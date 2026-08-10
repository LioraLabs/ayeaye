//! SHA-1, because a diff token has to mean the same thing in both daemons.
//!
//! The terminal view's tokens are opaque to the client: it is handed two and
//! echoes them back, and nothing in `share/app.html` looks inside them. So the
//! hash could have been anything — except that `bin/ayeaye` already mints them
//! as `sha1(text)[:12]`, and minting them the same way is free. A phone
//! mid-session can be pointed at either daemon without a full resend, and the
//! token stays a value the two implementations can be *compared* on rather than
//! one each invented.
//!
//! This is not here for its cryptography and must not be reached for as though
//! it were: SHA-1's collision resistance is broken, and the one property being
//! used is that the same text hashes to the same digest in Python and here.
//!
//! Hand-rolled because the core declares no dependencies at all — see
//! `crates/constitution/README.md` — and because the algorithm is a hundred
//! lines of shifts with published test vectors to hold it to.

/// The lower-case hex digest of SHA-1 over these bytes.
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(40);
    for word in digest(bytes) {
        out.push_str(&format!("{word:08x}"));
    }
    out
}

/// The five words of the digest, in order.
fn digest(bytes: &[u8]) -> [u32; 5] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];

    // The message, then a 1 bit, then zeros, then the length in bits as a
    // big-endian u64 — padded so the whole thing is a multiple of 64 bytes.
    // Built as one buffer rather than streamed: what is hashed here is a screen
    // full of text, and clarity is worth more than the copy.
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    // `as u64` before the shift: a message long enough to overflow this is one
    // no capture-pane will ever produce, and 2^61 bytes is the algorithm's own
    // limit anyway.
    padded.extend_from_slice(&((bytes.len() as u64) * 8).to_be_bytes());

    for block in padded.chunks_exact(64) {
        let mut schedule = [0u32; 80];
        for (index, word) in block.chunks_exact(4).enumerate() {
            schedule[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            schedule[index] = (schedule[index - 3]
                ^ schedule[index - 8]
                ^ schedule[index - 14]
                ^ schedule[index - 16])
                .rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in schedule.iter().enumerate() {
            let (mixed, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(mixed)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (slot, add) in state.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(add);
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::hex;

    // AYEAYE-47 — the published FIPS-180 vectors. Transcribed from the standard
    // rather than produced by this code, which is the only way a hash test says
    // anything: a digest compared against what the implementation happened to
    // return agrees with itself no matter what it computes.
    #[test]
    fn the_published_vectors_hash_to_their_published_digests() {
        assert_eq!(hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        // 56 bytes: the padding lands exactly on the length field's boundary,
        // which is the case an off-by-one in the pad loop gets wrong.
        assert_eq!(
            hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 112 bytes: two blocks, so a length that is only correct for the first
        // one shows up here and nowhere above.
        assert_eq!(
            hex(b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmn\
                  hijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu"),
            "a49b2446a02c645bf419f995b67091253a04a259"
        );
    }

    // AYEAYE-47 — the digest is 40 lower-case hex characters whatever it is
    // hashing, because the first twelve of them become a token in a URL.
    #[test]
    fn a_digest_is_forty_lower_case_hex_characters() {
        for text in ["", "a screen full of text\n", "café ✓\u{1b}[31m"] {
            let digest = hex(text.as_bytes());
            assert_eq!(digest.len(), 40, "for {text:?}");
            assert!(
                digest
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
                "{digest} is not lower-case hex"
            );
        }
        // Bytes that are not text hash too: a capture decoded lossily can carry
        // anything, and the hash is over what is about to be sent.
        assert_eq!(hex(&[0u8, 0xff, 0x80]).len(), 40);
    }

    // AYEAYE-47 — a hash that ignored part of its input would still pass the
    // vectors above if the vectors were the only test. One byte changed
    // anywhere has to change the digest.
    #[test]
    fn one_changed_byte_changes_the_digest() {
        let same = hex(b"the pane did not change");
        assert_eq!(same, hex(b"the pane did not change"));
        assert_ne!(same, hex(b"the pane did nol change"));
        assert_ne!(hex(b"ab"), hex(b"ba"));
        // Including in a later block, which is where a loop that only hashed
        // the first one would agree.
        let long = "x".repeat(200);
        let mut tail_changed = long.clone();
        tail_changed.push('y');
        assert_ne!(hex(long.as_bytes()), hex(tail_changed.as_bytes()));
    }
}
