# Design: Generalising the Sumcheck Evaluation Domain

## Problem

The sumcheck protocol is generic over `F: Field` and works correctly over binary extension
fields for degree-1 round polynomials (single-MLE products). It breaks for degree ≥ 2 because
`interpolate_uni_poly` and the prover's evaluation loop both use the integer sequence
`{0, 1, 2, ...}` as evaluation domain points, casting them into the field via `F::from(n)`.
In characteristic 2, `F::from(2) = 0 = F::from(0)`, so the points collide and Lagrange
interpolation divides by zero.

The fix is to replace `F::from(n)` with a field-aware lookup so each field type can supply
genuinely distinct evaluation points.

---

## Design

### New trait: `SumcheckField`

Add a trait that extends `Field` with a single method that maps an index to a domain point.

```rust
// src/field_ext.rs
pub trait SumcheckField: ark_ff::Field {
    fn eval_point(i: usize) -> Self {
        Self::from(i as u64)   // default: works for prime fields
    }
}
```

- Prime fields (`Fr`, `Fq`, …) implement it with a single empty `impl` — the default uses
  `F::from(i as u64)`, which embeds integers distinctly because `char >> degree`.
- Binary fields override it with a bit-pattern embedding (see below).

### Implementation for binary fields

`Gf128 = BinaryField<Gf128Config, 2>` stores elements as `[u64; 2]` limbs.
Constructing `BinaryField::new([i as u64, 0])` gives the element whose polynomial
representation is the integer `i` read as a bit string — genuinely distinct for any
practical degree (up to 2^64 points before collision).

```rust
// in binary-fields or in the sumcheck test file
impl SumcheckField for Gf128 {
    fn eval_point(i: usize) -> Self {
        Self::new([i as u64, 0])
    }
}
```

`eval_point(0)` = 0, `eval_point(1)` = 1, `eval_point(2)` = α, `eval_point(3)` = α+1, …
All distinct, no collision.

---

## Files to change

### 1. `src/field_ext.rs` — new file

Define the `SumcheckField` trait as shown above.

### 2. `src/lib.rs`

Add `pub mod field_ext;` and `pub use field_ext::SumcheckField;`.

### 3. `src/ml_sumcheck/protocol/mod.rs`

Change the bound on `IPForMLSumcheck`:

```
// before
pub struct IPForMLSumcheck<F: Field>

// after
pub struct IPForMLSumcheck<F: SumcheckField>
```

### 4. `src/ml_sumcheck/protocol/prover.rs`

**Import**: replace `use ark_ff::Field` with `use crate::SumcheckField`.

**Bound**: `impl<F: Field>` → `impl<F: SumcheckField>`.

**Evaluation loop** — the inner loop that evaluates the round polynomial at each domain point.
Currently uses integer increment (`start += step`) which only works for consecutive integers.
Replace with a direct evaluation at `eval_point(t)`:

```rust
// before
let mut start = table[b << 1];
let step = table[(b << 1) + 1] - start;
for p in product.iter_mut() {
    *p *= start;
    start += step;
}

// after
let f0 = table[b << 1];
let step = table[(b << 1) + 1] - f0;
for (t, p) in product.iter_mut().enumerate() {
    *p *= f0 + F::eval_point(t) * step;
}
```

The formula `f0 + eval_point(t) * step` is the multilinear extension evaluated at `t`:
`f(t) = f(0) + t * (f(1) - f(0))`. For t=0 and t=1 it is identical to the original.
For t=2 it uses `eval_point(2)` which is `2` in a prime field and `α` in a binary field.

### 5. `src/ml_sumcheck/protocol/verifier.rs`

**Import**: replace `use ark_ff::Field` with `use crate::SumcheckField`.

**Bound on `IPForMLSumcheck`**: `impl<F: Field>` → `impl<F: SumcheckField>`.

**`interpolate_uni_poly`**: replace the current three-branch implementation (u64, u128, BigInt
factorial denominators) with a single general Lagrange interpolation that uses `eval_point`.
The current optimisation is only valid for consecutive-integer domains:

```rust
// after — general Lagrange interpolation over any domain
pub(crate) fn interpolate_uni_poly<F: SumcheckField>(p_i: &[F], eval_at: F) -> F {
    let len = p_i.len();

    // early exit if eval_at is one of the domain points
    for i in 0..len {
        if eval_at == F::eval_point(i) {
            return p_i[i];
        }
    }

    // prod = ∏_{j} (eval_at - domain[j])
    let prod: F = (0..len).map(|j| eval_at - F::eval_point(j)).product();

    let mut result = F::zero();
    for i in 0..len {
        // numerator: prod / (eval_at - domain[i])
        let num = prod / (eval_at - F::eval_point(i));
        // denominator: ∏_{j≠i} (domain[i] - domain[j])
        let denom: F = (0..len)
            .filter(|&j| j != i)
            .map(|j| F::eval_point(i) - F::eval_point(j))
            .product();
        result += p_i[i] * num / denom;
    }
    result
}
```

All denominators are non-zero as long as the domain points are distinct, which is guaranteed
by the `SumcheckField` contract.

### 6. `src/ml_sumcheck/mod.rs` and `src/gkr_round_sumcheck/mod.rs`

Propagate the bound: anywhere `F: Field` is used on `MLSumcheck`, `GKRRoundSumcheck`, or
their `impl` blocks, change to `F: SumcheckField`.

### 7. Test files

Add explicit `SumcheckField` impls for the field types used in tests:

```rust
// Fr uses the default integer domain — no body needed
impl SumcheckField for Fr {}

// Gf128 uses bit-pattern embedding
impl SumcheckField for Gf128 {
    fn eval_point(i: usize) -> Self {
        Self::new([i as u64, 0])
    }
}
```

After this, the `(4, 9)` range works for `Gf128` and the existing `Fr` tests are unchanged.

---

## What does not change

- The sumcheck protocol logic — folding, challenges, subclaim structure.
- `ListOfProductsOfPolynomials`, `ProverState`, `VerifierState`, `Proof` — all keep `F: Field`.
- The `fix_variables` path — already correct for any field.
- The GKR test using `Fr` — passes unchanged with the empty impl.
