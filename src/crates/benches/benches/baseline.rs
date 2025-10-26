use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn baseline_bench(c: &mut Criterion) {
    c.bench_function("black_box_noop", |b| b.iter(|| black_box(())));
}

criterion_group!(benches, baseline_bench);
criterion_main!(benches);
