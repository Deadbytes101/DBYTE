use crate::diagnostics::{Diagnostic, DiagnosticCategory};

pub const DBUP_MAGIC: &[u8; 8] = b"DBUP1\0\0\0";
pub const DBUP_VERSION: u32 = 1;
pub const DBUP_HEADER_SIZE: usize = 64;
pub const DBUP_ENTRY_SIZE: usize = 64;
pub const DBUP_MAX_ENTRIES: usize = 16;
pub const DBUP_MAX_SIZE: usize = 64 * 1024;
pub const DBUP_MAX_NAME: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleInput {
    pub name: String,
    pub payload: Vec<u8>,
}

pub fn pack_user_bundle(inputs: &[BundleInput]) -> Result<Vec<u8>, Diagnostic> {
    if inputs.is_empty() || inputs.len() > DBUP_MAX_ENTRIES {
        return Err(invalid("DBUP1 requires between 1 and 16 entries"));
    }
    let mut inputs = inputs.to_vec();
    inputs.sort_by(|left, right| left.name.cmp(&right.name));
    for (index, input) in inputs.iter().enumerate() {
        validate_name(&input.name)?;
        if input.payload.is_empty() || input.payload.len() > 4096 {
            return Err(invalid(format!(
                "DBUP1 payload '{}' must contain 1..4096 bytes",
                input.name
            )));
        }
        if index != 0 && inputs[index - 1].name == input.name {
            return Err(invalid(format!("duplicate DBUP1 name '{}'", input.name)));
        }
    }
    let table_end = DBUP_HEADER_SIZE
        .checked_add(
            inputs
                .len()
                .checked_mul(DBUP_ENTRY_SIZE)
                .ok_or_else(|| invalid("DBUP1 table overflow"))?,
        )
        .ok_or_else(|| invalid("DBUP1 table overflow"))?;
    let payload_start = align_up(table_end, 16)?;
    let mut offsets = Vec::with_capacity(inputs.len());
    let mut end = payload_start;
    for input in &inputs {
        end = align_up(end, 16)?;
        offsets.push(end);
        end = end
            .checked_add(input.payload.len())
            .ok_or_else(|| invalid("DBUP1 payload overflow"))?;
    }
    if end > DBUP_MAX_SIZE {
        return Err(invalid("DBUP1 bundle exceeds 64 KiB"));
    }
    let mut output = vec![0u8; end];
    output[0..8].copy_from_slice(DBUP_MAGIC);
    put_u32(&mut output, 8, DBUP_VERSION)?;
    put_u32(&mut output, 12, inputs.len() as u32)?;
    put_u32(&mut output, 16, DBUP_HEADER_SIZE as u32)?;
    put_u32(&mut output, 20, DBUP_ENTRY_SIZE as u32)?;
    put_u32(&mut output, 24, payload_start as u32)?;
    put_u32(&mut output, 28, end as u32)?;
    for (index, input) in inputs.iter().enumerate() {
        let record = DBUP_HEADER_SIZE + index * DBUP_ENTRY_SIZE;
        put_u32(&mut output, record, input.name.len() as u32)?;
        put_u32(&mut output, record + 4, offsets[index] as u32)?;
        put_u32(&mut output, record + 8, input.payload.len() as u32)?;
        output[record + 12..record + 44].copy_from_slice(&sha256(&input.payload));
        output[record + 44..record + 44 + input.name.len()].copy_from_slice(input.name.as_bytes());
        output[offsets[index]..offsets[index] + input.payload.len()]
            .copy_from_slice(&input.payload);
    }
    Ok(output)
}

fn validate_name(name: &str) -> Result<(), Diagnostic> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > DBUP_MAX_NAME
        || bytes
            .iter()
            .any(|byte| !matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_'))
    {
        return Err(invalid(format!("invalid DBUP1 program name '{name}'")));
    }
    Ok(())
}

fn align_up(value: usize, alignment: usize) -> Result<usize, Diagnostic> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| invalid("DBUP1 alignment overflow"))
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), Diagnostic> {
    let target = bytes
        .get_mut(offset..offset + 4)
        .ok_or_else(|| invalid("DBUP1 write outside bundle"))?;
    target.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn invalid(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidObject, message)
}

pub fn sha256(input: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let mut state = INITIAL;
    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut output = [0u8; 32];
    for (chunk, word) in output.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        chunk.copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
    }

    #[test]
    fn bundle_is_canonical_and_bounded() {
        let a = BundleInput {
            name: "B.ELF".into(),
            payload: vec![2, 3],
        };
        let b = BundleInput {
            name: "A.ELF".into(),
            payload: vec![1],
        };
        assert_eq!(
            pack_user_bundle(&[a.clone(), b.clone()]).unwrap(),
            pack_user_bundle(&[b, a]).unwrap()
        );
    }

    #[test]
    fn bundle_rejects_duplicate_invalid_and_oversized_inputs() {
        let duplicate = BundleInput {
            name: "A.ELF".into(),
            payload: vec![1],
        };
        assert!(pack_user_bundle(&[duplicate.clone(), duplicate]).is_err());
        assert!(pack_user_bundle(&[BundleInput {
            name: "bad/name".into(),
            payload: vec![1],
        }])
        .is_err());
        assert!(pack_user_bundle(&[BundleInput {
            name: "BIG.ELF".into(),
            payload: vec![0; DBUP_MAX_SIZE],
        }])
        .is_err());
    }
}
