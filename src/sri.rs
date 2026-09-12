//! Subresource integrity — the pure half.
//!
//! One job: turn the bytes this server is about to serve into the exact `integrity="sha384-…"`
//! string a browser will check them against. Doing it from the served bytes, at boot, is the
//! point: a hash that is transcribed into a template by hand is a hash that goes stale the first
//! time somebody re-minifies a vendored file, and a stale SRI attribute fails *closed* in a way
//! that looks like an outage rather than a mistake.
//!
//! It is `std`-only so it can be compiled and tested on its own, which is how the digest was
//! checked against the FIPS 180-4 vectors and against `openssl dgst -sha384` before it was ever
//! wired to a response:
//!
//! ```text
//! rustc --edition 2021 --test src/sri.rs -o /tmp/sri && /tmp/sri
//! ```
//!
//! SHA-384 rather than SHA-256 because that is what the vendoring pin files record, and because
//! the truncated-SHA-512 construction is not vulnerable to length extension — irrelevant for a
//! subresource hash, but it costs nothing to pick the better primitive.

// ---------------------------------------------------------------------------------------------
// SHA-384 (FIPS 180-4): SHA-512's compression function, a different initial state, truncated to
// 48 bytes.
// ---------------------------------------------------------------------------------------------

const K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

const INITIAL_STATE: [u64; 8] = [
    0xcbbb9d5dc1059ed8,
    0x629a292a367cd507,
    0x9159015a3070dd17,
    0x152fecd8f70e5939,
    0x67332667ffc00b31,
    0x8eb44a8768581511,
    0xdb0c2e0d64f98fa7,
    0x47b5481dbefa4fa4,
];

/// The SHA-384 digest of `input`, as 48 bytes.
#[must_use]
pub fn sha384(input: &[u8]) -> [u8; 48] {
    let mut state = INITIAL_STATE;

    // Padding: 0x80, then zeros, then the message length in bits as a 128-bit big-endian integer.
    // A 128-bit length is more than this process will ever hash, so the high half is always zero;
    // it is still written, because a block that is one byte short of correct is a silent wrong
    // answer rather than an error.
    let mut padded = Vec::with_capacity(input.len() + 145);
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 128 != 112 {
        padded.push(0);
    }
    let bits = (input.len() as u128).wrapping_mul(8);
    padded.extend_from_slice(&bits.to_be_bytes());

    for block in padded.chunks_exact(128) {
        compress(&mut state, block);
    }

    let mut out = [0u8; 48];
    for (index, word) in state.iter().take(6).enumerate() {
        out[index * 8..index * 8 + 8].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn compress(state: &mut [u64; 8], block: &[u8]) {
    debug_assert_eq!(block.len(), 128);
    let mut w = [0u64; 80];
    for (index, chunk) in block.chunks_exact(8).enumerate() {
        w[index] = u64::from_be_bytes([
            chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6], chunk[7],
        ]);
    }
    for index in 16..80 {
        let s0 =
            w[index - 15].rotate_right(1) ^ w[index - 15].rotate_right(8) ^ (w[index - 15] >> 7);
        let s1 =
            w[index - 2].rotate_right(19) ^ w[index - 2].rotate_right(61) ^ (w[index - 2] >> 6);
        w[index] = w[index - 16]
            .wrapping_add(s0)
            .wrapping_add(w[index - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for index in 0..80 {
        let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[index])
            .wrapping_add(w[index]);
        let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

// ---------------------------------------------------------------------------------------------
// Base64
// ---------------------------------------------------------------------------------------------

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding — the only encoding an `integrity` attribute may use.
#[must_use]
pub fn base64(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[triple as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The value of an `integrity` attribute for `bytes`: `sha384-<base64>`.
#[must_use]
pub fn integrity(bytes: &[u8]) -> String {
    format!("sha384-{}", base64(&sha384(bytes)))
}

/// Whether `bytes` matches a pin recorded in an `assets/<name>.sha384` file.
///
/// The pin file is compared after trimming, and a `sha384-` prefix is optional so the same file
/// can be pasted straight into a template or read by `shasum -b -a 384 | base64` tooling.
#[must_use]
pub fn matches_pin(bytes: &[u8], pin: &str) -> bool {
    let pin = pin.trim();
    let pin = pin.strip_prefix("sha384-").unwrap_or(pin);
    let actual = base64(&sha384(bytes));
    // Not a secret, so a plain comparison is fine — but keep it length-first for the same reason
    // every other comparison in this crate is: a habit that is only sometimes applied is not one.
    pin.len() == actual.len()
        && pin
            .as_bytes()
            .iter()
            .zip(actual.as_bytes())
            .all(|(a, b)| a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// FIPS 180-4 / NIST CAVP vectors. If this file is ever "optimised", these are the guard.
    #[test]
    fn sha384_matches_the_published_vectors() {
        assert_eq!(
            hex(&sha384(b"")),
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"
        );
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&sha384(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu")),
            "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039"
        );
    }

    /// The four lengths where a hand-written padding loop goes wrong: one byte short of the
    /// length field, exactly against it, one byte short of a whole block, and exactly one block.
    #[test]
    fn padding_is_right_at_every_block_boundary() {
        for (length, expected) in [
            (111usize, "3c37955051cb5c3026f94d551d5b5e2ac38d572ae4e07172085fed81f8466b8f90dc23a8ffcdea0b8d8e58e8fdacc80a"),
            (112, "187d4e07cb306103c69967bf544d0dfbe9042577599c73c330abc0cb64c61236d5ed565ee19119d8c31779a38f791fcd"),
            (127, "9bd06b1763c2cf7aef40e795dc65bc96d59c41b537f3ad72ebdefd485476b5717c1aeb37c327fe9c1831b12b9efd08ae"),
            (128, "edb12730a366098b3b2beac75a3bef1b0969b15c48e2163c23d96994f8d1bef760c7e27f3c464d3829f56c0d53808b0b"),
        ] {
            assert_eq!(hex(&sha384(&vec![b'a'; length])), expected, "length {length}");
        }
        // 1 000 000 'a' is the classic long-message vector, and the only test here that exercises
        // more than a handful of compression rounds.
        assert_eq!(
            hex(&sha384(&vec![b'a'; 1_000_000])),
            "9d0e1809716474cb086e834e310a4a1ced149e9c00f248527972cec5704c2a5b07b8b3dc38ecc4ebae97ddd87f3d8985"
        );
    }

    #[test]
    fn base64_matches_rfc_4648() {
        for (input, expected) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input), expected, "input {input:?}");
        }
        // Every byte value round-trips through the alphabet without panicking.
        let all: Vec<u8> = (0..=255u8).collect();
        assert_eq!(base64(&all).len(), 344);
    }

    #[test]
    fn an_integrity_attribute_is_the_algorithm_then_the_digest() {
        let value = integrity(b"");
        assert!(value.starts_with("sha384-"));
        // 48 bytes → 64 base64 characters, no padding.
        assert_eq!(value.len(), "sha384-".len() + 64);
        assert!(!value.ends_with('='));
        assert_eq!(
            value,
            "sha384-OLBgp1GsljhM2TJ+sbHjaiH9txEUvgdDTAzHv2P24donTt6/529l+9Ua0vFImLlb"
        );
    }

    #[test]
    fn a_pin_matches_only_the_bytes_it_was_taken_from() {
        let bytes = b"console.log('hello');";
        let pin = integrity(bytes);
        assert!(matches_pin(bytes, &pin));
        assert!(matches_pin(bytes, pin.strip_prefix("sha384-").unwrap()));
        assert!(matches_pin(bytes, &format!("  {pin}\n")));
        // One byte different anywhere is a different file.
        assert!(!matches_pin(b"console.log('hello') ;", &pin));
        assert!(!matches_pin(bytes, "sha384-"));
        assert!(!matches_pin(bytes, ""));
    }
}
