use ark_ff::Field;
use ark_poly::DenseMultilinearExtension;
use ark_std::{rc::Rc, test_rng};
use ark_std::rand::RngCore;
use ark_sumcheck::ml_sumcheck::{MLSumcheck, data_structures::ListOfProductsOfPolynomials};
use ark_test_curves::bls12_381::Fr;
use binary_fields::ark::configs::gf128::Gf128;
use binary_fields::hekate::{Block128Ark, Block128FlatArk};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

// ── shared setup ─────────────────────────────────────────────────────────────

fn build_poly<F: Field, R: RngCore>(
    nv: usize,
    num_products: usize,
    rng: &mut R,
) -> (ListOfProductsOfPolynomials<F>, F) {
    let mut sum = F::zero();
    let mut poly = ListOfProductsOfPolynomials::new(nv);
    for _ in 0..num_products {
        // single MLE per term (degree-1): works for all three field types
        let evals: Vec<F> = (0..1 << nv).map(|_| F::rand(rng)).collect();
        let mle = Rc::new(DenseMultilinearExtension::from_evaluations_vec(nv, evals.clone()));
        let coeff = F::rand(rng);
        poly.add_product(core::iter::once(mle), coeff);
        sum += evals.iter().fold(F::zero(), |a, &v| a + v) * coeff;
    }
    (poly, sum)
}

fn bench_prove_verify<F: Field>(c: &mut Criterion, group_name: &str, nv: usize) {
    let mut rng = test_rng();
    let (poly, claimed_sum) = build_poly::<F, _>(nv, 5, &mut rng);

    let mut g = c.benchmark_group(group_name);

    g.bench_with_input(BenchmarkId::new("prove", nv), &nv, |b, _| {
        b.iter(|| {
            MLSumcheck::prove(black_box(&poly)).unwrap()
        })
    });

    let proof = MLSumcheck::prove(&poly).unwrap();
    let poly_info = poly.info();

    g.bench_with_input(BenchmarkId::new("verify", nv), &nv, |b, _| {
        b.iter(|| {
            MLSumcheck::verify(black_box(&poly_info), black_box(claimed_sum), black_box(&proof)).unwrap()
        })
    });

    g.finish();
}

// ── benchmarks ───────────────────────────────────────────────────────────────

fn bench_fr(c: &mut Criterion) {
    for nv in [8, 12, 16] {
        bench_prove_verify::<Fr>(c, "Fr (BLS12-381)", nv);
    }
}

fn bench_gf128(c: &mut Criterion) {
    for nv in [8, 12, 16] {
        bench_prove_verify::<Gf128>(c, "Gf128 (binary-fields)", nv);
    }
}

fn bench_block128ark(c: &mut Criterion) {
    for nv in [8, 12, 16] {
        bench_prove_verify::<Block128Ark>(c, "Block128Ark (hekate tower)", nv);
    }
}

fn bench_block128flatark(c: &mut Criterion) {
    for nv in [8, 12, 16, 20] {
        bench_prove_verify::<Block128FlatArk>(c, "Block128FlatArk (hekate flat/PMULL)", nv);
    }
}

criterion_group!(benches, bench_fr, bench_gf128, bench_block128ark, bench_block128flatark);
criterion_main!(benches);
