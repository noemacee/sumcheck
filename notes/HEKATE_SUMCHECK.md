# Design: Hekate-Math as a Sumcheck Field

## Context

`binary-fields` was bridged to `ark-sumcheck` by implementing `ark_ff::Field` directly on a
new generic type `BinaryField<P, N>`.

`hekate-math` has a parallel goal but is an independent crate with its own `TowerField` trait,
no `ark-ff` dependency, and six concrete field types (`Bit`, `Block8`, …, `Block128`).  The
task is to make at least `Block128` usable as the field type in sumcheck.

**Scope**: degree-1 round polynomials only (`num_multiplicands_range = (1, 2)`).  The
evaluation domain problem (degree ≥ 2 over char-2 fields) is a known limitation, deferred.

---

## Gap Analysis

### What hekate already has

| Requirement | Status | Notes |
|---|---|---|
| Field arithmetic (`+`, `-`, `*`, `/`) | ✓ | XOR add, Karatsuba mul |
| Multiplicative inverse | ✓ | `Block128::invert()` returns `Self` (0 for 0) |
| `Copy + Clone + Default + PartialEq + Eq` | ✓ | |
| `From<u64>` | ✓ (wrong semantics) | Does `val as u128` — bit-pattern embedding, not ring map |
| Serialization | ✓ (wrong trait) | Hekate's own `CanonicalSerialize`, not ark-serialize's |
| Random sampling | ✓ (wrong trait) | Hekate's own RNG, not `UniformRand` |
| `Hash` | ✗ | Not derived |
| `Ord / PartialOrd` | ✗ | Not implemented |
| `Zeroize` | ✓ | Already derived |
| `ark_ff::Field` | ✗ | Not implemented |
| `ark_ff::AdditiveGroup` | ✗ | Not implemented |

### Why `From<u64>` has wrong semantics for ark-ff

Hekate does `Block128(val as u128)` — bit-pattern embedding.  The ark-ff contract requires
`From<u64>` to be the **ring homomorphism** `n * 1_F`, which in char 2 maps every integer
to `n mod 2` (0 or 1).  The wrapper must fix this.

### Why modifying hekate directly is wrong

- Changing `From<u64>` to the ring map would break hekate's own algorithms that rely on
  the current bit-pattern semantics.
- Hekate intentionally has no ark-ff dependency; adding one changes its design contract.

---

## Solution: Thin Wrapper Type

Create a zero-cost wrapper around `Block128` that implements ark-ff traits, while delegating
all arithmetic to hekate's optimized implementation.  Hekate is untouched.

```
                ┌─────────────────────────────────────────┐
                │  sumcheck (dev-dependencies)             │
                │                                          │
                │   Block128Ark(Block128)                  │
                │   ├── impl ark_ff::Field                 │
                │   └── delegates arithmetic to Block128   │
                └─────────────────────────────────────────┘
                           ↓ wraps
                ┌─────────────────────────────────────────┐
                │  hekate-math (unchanged)                 │
                │   Block128(u128)                         │
                │   └── impl TowerField                    │
                └─────────────────────────────────────────┘
```

---

## Files to Create / Change

### 1. `Cargo.toml` of sumcheck

Add hekate to dev-dependencies:

```toml
[dev-dependencies]
hekate-math = { path = "../../hekate-math" }
```

Note: `hekate-math` uses `rand = "0.10"`.  Check that this resolves to the same version
`ark-std` pulls in.  A `[patch.crates-io]` section may be needed if they conflict.

### 2. `src/ml_sumcheck/test.rs`

#### 2a. The wrapper type

```rust
use hekate_math::towers::block128::Block128;

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
struct Block128Ark(Block128);
```

`Block128` wraps a `pub u128`, so `Block128Ark` is transparently a `u128` at runtime.

#### 2b. `Hash + Ord`

```rust
impl Hash for Block128Ark {
    fn hash<H: Hasher>(&self, state: &mut H) { self.0.0.hash(state); }
}
impl PartialOrd for Block128Ark {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for Block128Ark {
    fn cmp(&self, other: &Self) -> Ordering { self.0.0.cmp(&other.0.0) }
}
```

#### 2c. Arithmetic — delegate to Block128

```rust
impl Neg for Block128Ark {
    type Output = Self;
    fn neg(self) -> Self { self }   // -x = x in char 2
}

impl AddAssign<&Self> for Block128Ark {
    fn add_assign(&mut self, rhs: &Self) { self.0 += rhs.0; }  // XOR
}
// same pattern for Sub (also XOR), Mul, Div
// Div: if rhs.0.0 == 0 { panic! } else { self * rhs.invert() }
// provide all owned / &Self / &mut Self variants
```

#### 2d. `From<integer>` — ring map

```rust
impl From<u64> for Block128Ark {
    fn from(n: u64) -> Self {
        Self(Block128((n & 1) as u128))   // ring homomorphism: n mod 2
    }
}
// same for u8, u16, u32, u128, i8, i16, i32, i64, i128, bool
```

#### 2e. `Zeroize`

```rust
impl Zeroize for Block128Ark {
    fn zeroize(&mut self) { self.0.0 = 0; }
}
```

#### 2f. `Zero + One + Sum + Product`

```rust
impl Zero for Block128Ark {
    fn zero() -> Self { Self(Block128::ZERO) }
    fn is_zero(&self) -> bool { self.0.0 == 0 }
}
impl One for Block128Ark {
    fn one() -> Self { Self(Block128::ONE) }
}
// Sum: fold with Zero + add
// Product: fold with One + mul
```

#### 2g. `UniformRand`

```rust
impl Distribution<Block128Ark> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Block128Ark {
        let lo = rng.next_u64() as u128;
        let hi = rng.next_u64() as u128;
        Block128Ark(Block128(lo | (hi << 64)))
    }
}
```

#### 2h. ark-serialize `CanonicalSerialize / CanonicalDeserialize`

16 bytes, little-endian:

```rust
impl CanonicalSerialize for Block128Ark {
    fn serialize_with_mode<W: Write>(&self, mut w: W, _: Compress) -> Result<(), SerializationError> {
        w.write_all(&self.0.0.to_le_bytes()).map_err(SerializationError::IoError)
    }
    fn serialized_size(&self, _: Compress) -> usize { 16 }
}
impl CanonicalDeserialize for Block128Ark {
    fn deserialize_with_mode<R: Read>(mut r: R, _: Compress, _: Validate) -> Result<Self, SerializationError> {
        let mut buf = [0u8; 16];
        r.read_exact(&mut buf).map_err(SerializationError::IoError)?;
        Ok(Self(Block128(u128::from_le_bytes(buf))))
    }
}
// also impl Valid (trivial Ok(())), CanonicalSerializeWithFlags, CanonicalDeserializeWithFlags
```

#### 2i. `AdditiveGroup`

```rust
impl AdditiveGroup for Block128Ark {
    type Scalar = Self;
    const ZERO: Self = Self(Block128(0));
    fn double_in_place(&mut self) -> &mut Self {
        self.0.0 = 0;   // x + x = 0 in char 2
        self
    }
    fn neg_in_place(&mut self) -> &mut Self { self }
}
```

#### 2j. `Field`

```rust
impl Field for Block128Ark {
    type BasePrimeField = Gf2;   // reuse from binary-fields, already a dev-dep

    const SQRT_PRECOMP: Option<SqrtPrecomputation<Self>> = None;
    const ONE: Self = Self(Block128(1));

    fn extension_degree() -> u64 { 128 }
    fn characteristic() -> &'static [u64] { &[2] }

    fn from_base_prime_field(elem: Gf2) -> Self {
        Self(Block128(elem.0.0[0] as u128))
    }
    fn to_base_prime_field_elements(&self) -> impl Iterator<Item = Gf2> {
        (0..128).map(|i| Gf2::from(((self.0.0 >> i) & 1) as u64))
    }
    fn from_base_prime_field_elems(elems: impl IntoIterator<Item = Gf2>) -> Option<Self> {
        let mut val = 0u128;
        let mut count = 0usize;
        for (i, e) in elems.into_iter().enumerate() {
            if i >= 128 { return None; }
            val |= (e.0.0[0] as u128) << i;
            count += 1;
        }
        if count != 128 { return None; }
        Some(Self(Block128(val)))
    }

    fn inverse(&self) -> Option<Self> {
        if self.0.0 == 0 { return None; }
        Some(Self(self.0.invert()))   // hekate returns 0 for 0; we guard above
    }

    fn square(&self) -> Self { Self(self.0 * self.0) }
    fn square_in_place(&mut self) -> &mut Self {
        self.0 = self.0 * self.0; self
    }

    fn sqrt(&self) -> Option<Self> {
        // sqrt(a) = a^(2^127) in GF(2^128)
        let mut r = *self;
        for _ in 0..127 { r = r.square(); }
        Some(r)
    }

    fn legendre(&self) -> LegendreSymbol {
        if self.is_zero() { LegendreSymbol::Zero } else { LegendreSymbol::QuadraticResidue }
    }

    fn frobenius_map_in_place(&mut self, power: usize) {
        for _ in 0..power { *self = self.square(); }
    }

    fn from_random_bytes_with_flags<F: Flags>(bytes: &[u8]) -> Option<(Self, F)> {
        if bytes.len() < 16 { return None; }
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&bytes[..16]);
        let flag = F::from_u8(0)?;
        Some((Self(Block128(u128::from_le_bytes(buf))), flag))
    }

    fn mul_by_base_prime_field(&self, elem: &Gf2) -> Self {
        if elem.is_zero() { Self::zero() } else { *self }
    }
}
```

### 3. Tests in `src/ml_sumcheck/test.rs`

Mirror the binary-fields pattern, restricting to `(1, 2)` multiplicands:

```rust
fn test_polynomial_block128(nv: usize, num_multiplicands_range: (usize, usize), num_products: usize) { ... }
fn test_protocol_block128(nv: usize, num_multiplicands_range: (usize, usize), num_products: usize) { ... }

#[test] fn test_trivial_polynomial_block128()              // nv=1, (1,2), 5 products
#[test] fn test_normal_polynomial_block128()               // nv=12, (1,2), 5 products
#[test] fn test_normal_polynomial_block128_wrong_sum_rejected()
```

No high-degree test — that requires `GENERALISE_DOMAIN.md` work, which is deferred.

---

## What Does Not Change

- Hekate-math source is **untouched**
- Sumcheck protocol logic, prover, verifier — unchanged
- The `(1, 2)` multiplicands restriction is the same as the binary-fields baseline

---

## Known Limitation (Deferred)

Degree ≥ 2 round polynomials will panic (division by zero in `interpolate_uni_poly`) for
the same reason they do with `Gf128`: `F::from(2) = 0` in char 2.  Fixing this requires
the `SumcheckField` evaluation domain generalisation described in `GENERALISE_DOMAIN.md`.
