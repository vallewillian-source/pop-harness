pub const SCALE: f32 = 127.0;

pub fn quantize(vector: &[f32]) -> Vec<i8> {
    let largest = vector.iter().fold(0.0f32, |acc, x| acc.max(x.abs()));
    if largest == 0.0 || !largest.is_finite() {
        return vec![0; vector.len()];
    }
    let factor = SCALE / largest;
    vector
        .iter()
        .map(|component| (component * factor).round().clamp(-SCALE, SCALE) as i8)
        .collect()
}

pub fn norm(vector: &[i8]) -> f32 {
    let sum: i64 = vector.iter().map(|x| i64::from(*x) * i64::from(*x)).sum();
    (sum as f32).sqrt()
}

pub fn dot_product(a: &[i8], b: &[i8]) -> i32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| i32::from(*x) * i32::from(*y))
        .sum()
}

pub fn cosine(product: i32, norm_a: f32, norm_b: f32) -> f32 {
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        product as f32 / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantization_preserves_direction() {
        assert_eq!(quantize(&[1.0, -1.0, 0.0, 0.5]), vec![127, -127, 0, 64]);
        assert_eq!(quantize(&[0.02, -0.03]), quantize(&[2.0, -3.0]));
        assert_eq!(quantize(&[2.0, -3.0]), vec![85, -127]);
        assert_eq!(quantize(&[0.0, 0.0]), vec![0, 0]);
    }

    #[test]
    fn cosine_of_equal_and_zero_vectors() {
        let v = quantize(&[0.6, 0.8]);
        let n = norm(&v);
        assert!((cosine(dot_product(&v, &v), n, n) - 1.0).abs() < 1e-3);
        let z = vec![0i8; 2];
        assert_eq!(cosine(dot_product(&v, &z), n, norm(&z)), 0.0);
    }
}
