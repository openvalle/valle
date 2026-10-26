// ES 3.00 has no fused multiply-add. Recover the division residual with exact 12-bit
// products and cancelling subtractions, before discontinuous alpha, PCF and DOF decisions.
fn web_divide(a:f32,b:f32,q:f32)->f32 {
    let qh=bitcast<f32>(bitcast<u32>(q)&0xfffff000u);
    let bh=bitcast<f32>(bitcast<u32>(b)&0xfffff000u);
    let ql=stable(q-qh);let bl=stable(b-bh);
    let r0=stable(a-stable(qh*bh));
    let r1=stable(r0-stable(ql*bh));
    let r2=stable(r1-stable(qh*bl));
    let residual=stable(r2-stable(ql*bl));
    return stable(q+stable(residual/b));
}
