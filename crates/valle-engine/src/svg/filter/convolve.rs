//! Parameters for SVG's spatial convolution. The kernel is reversed, not correlated.
use super::Attributes;

#[derive(Clone, Debug, PartialEq)]
pub struct Kernel {
    pub order: [u32; 2],
    pub values: Vec<f64>,
    pub divisor: f64,
    pub bias: f64,
    pub target: [u32; 2],
    pub edge: u32,
    pub unit: Option<[f64; 2]>,
    pub preserve_alpha: bool,
}

impl Kernel {
    pub(super) fn parse(a: &Attributes<'_>) -> Result<Self, String> {
        let order = a.pair("order", 3., true)?;
        if order
            .iter()
            .any(|n| n.fract() != 0. || !(1. ..=f64::from(u32::MAX)).contains(n))
        {
            return Err("order: expected one or two positive integers".into());
        }
        let order = order.map(|n| n as u32);
        let values = a
            .numbers("kernelMatrix")?
            .ok_or("feConvolveMatrix needs kernelMatrix")?;
        if u64::from(order[0]) * u64::from(order[1]) != values.len() as u64 {
            return Err("kernelMatrix: length must equal orderX * orderY".into());
        }
        let sum = values.iter().sum::<f64>();
        let divisor = a.number("divisor", if sum == 0. { 1. } else { sum })?;
        if divisor == 0. {
            return Err("divisor: must not be zero".into());
        }
        let mut target = [0; 2];
        for (i, name) in ["targetX", "targetY"].into_iter().enumerate() {
            let n = a.number(name, f64::from(order[i] / 2))?;
            if n.fract() != 0. || n < 0. || n >= f64::from(order[i]) {
                return Err(format!("{name}: expected an integer within the kernel"));
            }
            target[i] = n as u32;
        }
        let unit = a
            .get("kernelUnitLength")
            .map(|_| a.pair("kernelUnitLength", 1., true))
            .transpose()?;
        if unit.is_some_and(|u| u.iter().any(|v| *v <= 0.)) {
            return Err("kernelUnitLength: must be positive".into());
        }
        Ok(Self {
            order,
            values,
            divisor,
            bias: a.number("bias", 0.)?,
            target,
            edge: a.choice("edgeMode", &["duplicate", "wrap", "none"], 0)?,
            unit,
            preserve_alpha: a.choice("preserveAlpha", &["false", "true"], 0)? == 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::V;

    fn parse(attrs: &[(&str, &str)]) -> Result<Kernel, String> {
        let values = attrs
            .iter()
            .map(|(k, v)| (*k, V::Str((*v).into())))
            .collect::<Vec<_>>();
        let refs = values.iter().map(|(k, v)| (*k, v)).collect::<Vec<_>>();
        Kernel::parse(&Attributes(&refs))
    }

    #[test]
    fn defaults_and_invalid_kernels() {
        let k = parse(&[("kernelMatrix", "0 1 0 1 4 1 0 1 0")]).unwrap();
        assert_eq!(k.divisor, 8.);
        assert_eq!(k.target, [1, 1]);
        assert_eq!(k.unit, None);
        assert_eq!(
            parse(&[("order", "2 1"), ("kernelMatrix", "-1 1")])
                .unwrap()
                .divisor,
            1.
        );
        for attrs in [
            vec![("order", "0"), ("kernelMatrix", "1")],
            vec![("order", "1.5"), ("kernelMatrix", "1")],
            vec![("order", "2"), ("kernelMatrix", "1")],
            vec![("order", "1"), ("kernelMatrix", "1"), ("divisor", "0")],
            vec![("order", "1"), ("kernelMatrix", "1"), ("targetX", "1")],
            vec![
                ("order", "1"),
                ("kernelMatrix", "1"),
                ("kernelUnitLength", "0"),
            ],
            vec![
                ("order", "1"),
                ("kernelMatrix", "1"),
                ("preserveAlpha", "yes"),
            ],
        ] {
            assert!(parse(&attrs).is_err(), "{attrs:?}");
        }
    }
}
