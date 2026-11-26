const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut saida = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let value = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        saida.push(char::from(ALPHABET[((value >> 18) & 0x3f) as usize]));
        saida.push(char::from(ALPHABET[((value >> 12) & 0x3f) as usize]));
        saida.push(if chunk.len() > 1 {
            char::from(ALPHABET[((value >> 6) & 0x3f) as usize])
        } else {
            '='
        });
        saida.push(if chunk.len() > 2 {
            char::from(ALPHABET[(value & 0x3f) as usize])
        } else {
            '='
        });
    }
    saida
}

#[cfg(test)]
mod tests {
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
