//! Codificação base64 (RFC 4648, com preenchimento), para enviar imagens como data URL.
//!
//! Feita aqui para não trazer um crate só para isto.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Codifica bytes em base64 padrão.
pub fn encode(bytes: &[u8]) -> String {
    let mut saida = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for bloco in bytes.chunks(3) {
        let b0 = bloco[0];
        let b1 = bloco.get(1).copied().unwrap_or(0);
        let b2 = bloco.get(2).copied().unwrap_or(0);
        let valor = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        saida.push(char::from(ALPHABET[((valor >> 18) & 0x3f) as usize]));
        saida.push(char::from(ALPHABET[((valor >> 12) & 0x3f) as usize]));
        saida.push(if bloco.len() > 1 {
            char::from(ALPHABET[((valor >> 6) & 0x3f) as usize])
        } else {
            '='
        });
        saida.push(if bloco.len() > 2 {
            char::from(ALPHABET[(valor & 0x3f) as usize])
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
    fn rfc_4648_vectors() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn high_bytes_use_full_alphabet() {
        assert_eq!(encode(&[0xfb, 0xff, 0xbf]), "+/+/");
    }
}
