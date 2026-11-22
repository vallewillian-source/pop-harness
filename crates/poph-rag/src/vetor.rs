//! Vetores int8 do índice (HR-16, D-09).
//!
//! Cada vetor é escalado pelo próprio maior componente em valor absoluto, e cada posição vira um
//! inteiro de -127 a 127. O cosseno não muda com a escala de cada vetor, então a busca não precisa
//! guardar o fator. Escalar pelo maior componente, e não por 1,0, aproveita os 255 níveis: os
//! componentes de um vetor de 384 posições são pequenos e uma escala fixa perderia quase toda a
//! resolução. A varredura faz produto interno exato sobre esses inteiros, sem índice aproximado.

/// Maior valor absoluto de um vetor quantizado.
pub const ESCALA: f32 = 127.0;

/// Quantiza um vetor de ponto flutuante para int8, escalando pelo maior componente. Vetor zerado
/// vira vetor zerado.
pub fn quantizar(vetor: &[f32]) -> Vec<i8> {
    let maior = vetor
        .iter()
        .fold(0.0f32, |acumulado, x| acumulado.max(x.abs()));
    if maior == 0.0 || !maior.is_finite() {
        return vec![0; vetor.len()];
    }
    let fator = ESCALA / maior;
    vetor
        .iter()
        .map(|componente| (componente * fator).round().clamp(-ESCALA, ESCALA) as i8)
        .collect()
}

/// Comprimento do vetor int8 (raiz da soma dos quadrados).
pub fn norma(vetor: &[i8]) -> f32 {
    let soma: i64 = vetor.iter().map(|x| i64::from(*x) * i64::from(*x)).sum();
    (soma as f32).sqrt()
}

/// Produto interno exato de dois vetores int8.
pub fn produto_interno(a: &[i8], b: &[i8]) -> i32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| i32::from(*x) * i32::from(*y))
        .sum()
}

/// Cosseno a partir do produto interno e dos dois comprimentos. Vetor zerado dá 0.
pub fn cosseno(produto: i32, norma_a: f32, norma_b: f32) -> f32 {
    if norma_a == 0.0 || norma_b == 0.0 {
        0.0
    } else {
        produto as f32 / (norma_a * norma_b)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn quantiza_pelo_maior_componente_e_mantem_a_direcao() {
        assert_eq!(quantizar(&[1.0, -1.0, 0.0, 0.5]), vec![127, -127, 0, 64]);
        // Mesma direção em outra escala dá o mesmo vetor int8.
        assert_eq!(quantizar(&[0.02, -0.03]), quantizar(&[2.0, -3.0]));
        assert_eq!(quantizar(&[2.0, -3.0]), vec![85, -127]);
        assert_eq!(quantizar(&[0.0, 0.0]), vec![0, 0]);
    }

    #[test]
    fn cosseno_de_vetor_igual_e_um_e_de_zerado_e_zero() {
        let v = quantizar(&[0.6, 0.8]);
        let n = norma(&v);
        assert!((cosseno(produto_interno(&v, &v), n, n) - 1.0).abs() < 1e-3);
        let z = vec![0i8; 2];
        assert_eq!(cosseno(produto_interno(&v, &z), n, norma(&z)), 0.0);
    }
}
