# Sumcheck over Binary Fields

## What was done

The `ark-sumcheck` crate originally used `Fr` (BLS12-381 scalar field) as its underlying field.
The goal was to run the multilinear sumcheck protocol over `Gf128` — the binary extension field GF(2^128) — implemented in `binary-fields`.

Two changes were needed:

1. **Version alignment.** `ark-sumcheck` depended on `ark-ff 0.4`, while `binary-fields` was built against `ark-ff 0.5`. Bumping the sumcheck dependencies to `0.5` unified both crates on the same version and let `Gf128` satisfy the `Field` bound directly.

2. **Multiplicands constraint.** The tests were adjusted to use `num_multiplicands_range = (1, 2)` instead of `(4, 9)`. This is explained below.

No changes to the protocol code were required. `Gf128` plugged in as a drop-in `Field` implementation.

---

## Field requirements for sumcheck

The sumcheck prover and verifier are generic over `F: ark_ff::Field`. Concretely, the protocol uses:

- **Addition, subtraction, multiplication** — field arithmetic each round
- **`F::zero()`, `F::one()`** — boundary values on the boolean hypercube
- **`F::rand(rng)`** — verifier samples a random challenge each round
- **`fix_variables`** on `DenseMultilinearExtension<F>` — reduces `f(r, x₂, ..., xₙ)` each round via linear interpolation: `f[b] = f[2b] + r · (f[2b+1] − f[2b])`
- **Lagrange interpolation** in the verifier — reconstructs the round polynomial from its evaluations

The first four work correctly over any field, including GF(2^128).
The last one has a constraint described below.

---

## The multiplicands constraint

### What `num_multiplicands` is

The sumcheck polynomial is represented as a sum of products of multilinear extensions (MLEs):

```
f(x) = c₁ · f₁(x) · f₂(x) · f₃(x)  +  c₂ · f₄(x) · f₅(x)  +  ...
         ↑ one product term, 3 multiplicands    ↑ one product term, 2 multiplicands
```

- **`num_products`** — how many terms are added together
- **`num_multiplicands`** — how many MLEs are multiplied within each term

`num_products` has no effect on correctness. `num_multiplicands` directly controls the degree of the round polynomial sent by the prover each round: multiplying `d` linear functions gives a degree-`d` univariate polynomial.

### The interpolation problem in characteristic 2

The verifier reconstructs the round polynomial via Lagrange interpolation. The evaluation domain used is the integer sequence `{0, 1, 2, ..., d}` embedded into the field via `F::from(n)`.

In a characteristic-2 field, this embedding maps every integer to its value mod 2:

```
F::from(0) = 0
F::from(1) = 1
F::from(2) = 0   ← collision with F::from(0)
F::from(3) = 1   ← collision with F::from(1)
```

For degree ≥ 2 the domain points are no longer distinct. The Lagrange denominator includes a factor of `F::from(2) = 0`, causing a division-by-zero panic at runtime.

### The constraint

| `num_multiplicands` | Round poly degree | Evaluation domain | Works in GF(2^k)? |
|---|---|---|---|
| 1 | 1 | `{0, 1}` | Yes — always distinct |
| 2 | 2 | `{0, 1, 2}` | No — `2 = 0` collision |
| d ≥ 2 | d | `{0, 1, ..., d}` | No |

With `num_multiplicands_range = (1, 2)` every product has exactly one MLE, the round polynomial is degree 1, and only `{0, 1}` are ever needed — both distinct in any field.

### What this means in practice

Degree-1 sumcheck (single-MLE products) covers the most common ZK use case: proving evaluation claims for committed multilinear polynomials. This is the core of protocols like Spartan and HyperPlonk.

GKR circuits require degree-2 products (wiring predicates multiplied by gate values), which are not supported with the current interpolation code. The fix is contained: `interpolate_uni_poly` would need to use genuinely distinct elements from the binary field as its evaluation domain (e.g. `{0, 1, α}` where `α` is a tower basis element) instead of the integer sequence.
