//! Codificação base64 (RFC 4648, com preenchimento), para enviar imagens como data URL.
//!
//! Feita aqui para não trazer um crate só para isto.

const ALFABETO: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Codifica bytes em base64 padrão.
pub fn codificar(bytes: &[u8]) -> String {
    let mut saida = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for bloco in bytes.chunks(3) {
        let b0 = bloco[0];
        let b1 = bloco.get(1).copied().unwrap_or(0);
        let b2 = bloco.get(2).copied().unwrap_or(0);
        let valor = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        saida.push(char::from(ALFABETO[((valor >> 18) & 0x3f) as usize]));
        saida.push(char::from(ALFABETO[((valor >> 12) & 0x3f) as usize]));
        saida.push(if bloco.len() > 1 {
            char::from(ALFABETO[((valor >> 6) & 0x3f) as usize])
        } else {
            '='
        });
        saida.push(if bloco.len() > 2 {
            char::from(ALFABETO[(valor & 0x3f) as usize])
        } else {
            '='
        });
    }
    saida
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn vetores_da_rfc_4648() {
        assert_eq!(codificar(b""), "");
        assert_eq!(codificar(b"f"), "Zg==");
        assert_eq!(codificar(b"fo"), "Zm8=");
        assert_eq!(codificar(b"foo"), "Zm9v");
        assert_eq!(codificar(b"foob"), "Zm9vYg==");
        assert_eq!(codificar(b"fooba"), "Zm9vYmE=");
        assert_eq!(codificar(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn bytes_altos_usam_o_alfabeto_inteiro() {
        assert_eq!(codificar(&[0xfb, 0xff, 0xbf]), "+/+/");
    }
}
