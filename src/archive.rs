use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use byteorder::{LittleEndian, ReadBytesExt};

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub name: String,
    pub file_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Archive {
    entries: HashMap<String, ArchiveEntry>,
}

impl Archive {
    pub fn from_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut cursor = std::io::Cursor::new(bytes);
        let mut entries = HashMap::new();

        while (cursor.position() as usize) < bytes.len() {
            let name = read_c_string(&mut cursor).context("failed to read entry name")?;
            let file_type = read_c_string(&mut cursor).context("failed to read entry type")?;

            if name.is_empty()
                && file_type.is_empty()
                && (cursor.position() as usize) == bytes.len()
            {
                break;
            }

            let flags = cursor.read_u32::<LittleEndian>()?;
            let compressed_len = cursor.read_u32::<LittleEndian>()? as usize;
            let uncompressed_len = cursor.read_u32::<LittleEndian>()? as usize;

            let start = cursor.position() as usize;
            let end = start
                .checked_add(compressed_len)
                .context("archive entry length overflow")?;
            if end > bytes.len() {
                bail!("archive entry {} exceeds input size", name);
            }

            let mut data = bytes[start..end].to_vec();
            cursor.set_position(end as u64);

            if flags & 1 != 0 {
                data = decompress(&data, uncompressed_len)
                    .with_context(|| format!("failed to decompress {}", name))?;
            }

            entries.insert(
                name.clone(),
                ArchiveEntry {
                    name,
                    file_type,
                    data,
                },
            );
        }

        Ok(Self { entries })
    }

    pub fn get(&self, name: &str) -> Option<&ArchiveEntry> {
        self.entries.get(name)
    }

    pub fn entries(&self) -> Vec<&ArchiveEntry> {
        let mut entries: Vec<_> = self.entries.values().collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }
}

fn read_c_string(cursor: &mut std::io::Cursor<&[u8]>) -> Result<String> {
    let bytes = cursor.get_ref();
    let start = cursor.position() as usize;
    let Some(rel_end) = bytes[start..].iter().position(|byte| *byte == 0) else {
        bail!("unterminated c-string");
    };
    let end = start + rel_end;
    let string_bytes = bytes[start..end].to_vec();
    cursor.set_position((end + 1) as u64);
    String::from_utf8(string_bytes).context("invalid utf-8 in c-string")
}

fn decompress(input: &[u8], output_len: usize) -> Result<Vec<u8>> {
    if input.is_empty() {
        bail!("empty compressed stream");
    }

    let mut output = vec![0u8; output_len];
    let mut table_offsets = [0usize; 4096];
    let mut table_lengths = [0usize; 4096];
    let mut next_code = 256usize;
    let mut write_index = 0usize;
    let mut prev_offset = 0usize;
    let mut prev_length = 1usize;

    output[write_index] = input[0];
    write_index += 1;

    let mut r = 1usize;
    loop {
        let packed_index = r + (r >> 1);
        if packed_index + 1 >= input.len() {
            break;
        }

        let m = input[packed_index + 1] as usize;
        let n = input[packed_index] as usize;
        let code = if r & 1 == 1 {
            (m << 4) | (n >> 4)
        } else {
            ((m & 15) << 8) | n
        };

        let (entry_offset, entry_length) = if code < next_code {
            if code < 256 {
                ensure_room(write_index, 1, output_len)?;
                output[write_index] = code as u8;
                let current_offset = write_index;
                write_index += 1;
                (current_offset, 1)
            } else {
                let current_offset = write_index;
                let length = table_lengths[code];
                let mut src = table_offsets[code];
                let end = src + length;
                ensure_room(write_index, length, output_len)?;
                while src < end {
                    output[write_index] = output[src];
                    write_index += 1;
                    src += 1;
                }
                (current_offset, length)
            }
        } else if code == next_code {
            let current_offset = write_index;
            let length = prev_length + 1;
            let mut src = prev_offset;
            let end = prev_offset + prev_length;
            ensure_room(write_index, length, output_len)?;
            while src < end {
                output[write_index] = output[src];
                write_index += 1;
                src += 1;
            }
            output[write_index] = output[prev_offset];
            write_index += 1;
            (current_offset, length)
        } else {
            break;
        };

        table_offsets[next_code] = prev_offset;
        table_lengths[next_code] = prev_length + 1;
        next_code += 1;
        prev_offset = entry_offset;
        prev_length = entry_length;
        if next_code >= 4096 {
            next_code = 256;
        }
        r += 1;
    }

    if write_index != output_len {
        bail!(
            "decompression length mismatch: expected {}, got {}",
            output_len,
            write_index
        );
    }

    Ok(output)
}

fn ensure_room(write_index: usize, count: usize, output_len: usize) -> Result<()> {
    if write_index + count > output_len {
        bail!("decompression overflow");
    }
    Ok(())
}
