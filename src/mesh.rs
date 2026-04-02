use std::io::Cursor;

use anyhow::{Context, Result, bail};
use byteorder::{LittleEndian, ReadBytesExt};

use crate::scene::MeshDesc;

#[derive(Debug, Clone)]
pub struct DecodedMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub bitangents: Vec<[f32; 3]>,
    pub texcoords: Vec<[f32; 2]>,
    pub secondary_texcoords: Option<Vec<[f32; 2]>>,
    pub colors: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
}

pub fn decode_mesh(bytes: &[u8], mesh: &MeshDesc) -> Result<DecodedMesh> {
    let mut cursor = Cursor::new(bytes);
    let mut all_indices = Vec::with_capacity(mesh.index_count);

    for _ in 0..mesh.index_count {
        let index = match mesh.index_type_size {
            2 => cursor.read_u16::<LittleEndian>()? as u32,
            4 => cursor.read_u32::<LittleEndian>()?,
            other => bail!("unsupported index size {}", other),
        };
        all_indices.push(index);
    }

    let wire_skip = mesh
        .wire_count
        .checked_mul(mesh.index_type_size)
        .context("wire count overflow")?;
    cursor.set_position(cursor.position() + wire_skip as u64);

    let has_secondary_uv = mesh.secondary_tex_coord.unwrap_or(0) > 0;
    let has_vertex_color = mesh.vertex_color.unwrap_or(0) > 0;

    let mut positions = Vec::with_capacity(mesh.vertex_count);
    let mut normals = Vec::with_capacity(mesh.vertex_count);
    let mut tangents = Vec::with_capacity(mesh.vertex_count);
    let mut bitangents = Vec::with_capacity(mesh.vertex_count);
    let mut texcoords = Vec::with_capacity(mesh.vertex_count);
    let mut secondary_texcoords = if has_secondary_uv {
        Some(Vec::with_capacity(mesh.vertex_count))
    } else {
        None
    };
    let mut colors = if has_vertex_color {
        Some(Vec::with_capacity(mesh.vertex_count))
    } else {
        None
    };

    for _ in 0..mesh.vertex_count {
        positions.push([
            cursor.read_f32::<LittleEndian>()?,
            cursor.read_f32::<LittleEndian>()?,
            cursor.read_f32::<LittleEndian>()?,
        ]);

        texcoords.push([
            cursor.read_f32::<LittleEndian>()?,
            -cursor.read_f32::<LittleEndian>()?,
        ]);

        if has_secondary_uv {
            let uv = [
                cursor.read_f32::<LittleEndian>()?,
                -cursor.read_f32::<LittleEndian>()?,
            ];
            if let Some(secondary) = &mut secondary_texcoords {
                secondary.push(uv);
            }
        }

        tangents.push(unpack_unit_vector(
            cursor.read_u16::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
        ));
        bitangents.push(unpack_unit_vector(
            cursor.read_u16::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
        ));
        normals.push(unpack_unit_vector(
            cursor.read_u16::<LittleEndian>()?,
            cursor.read_u16::<LittleEndian>()?,
        ));

        if let Some(vertex_colors) = &mut colors {
            let r = cursor.read_u8()? as f32 / 255.0;
            let g = cursor.read_u8()? as f32 / 255.0;
            let b = cursor.read_u8()? as f32 / 255.0;
            let a = cursor.read_u8()? as f32 / 255.0;
            vertex_colors.push([r, g, b, a]);
        }
    }

    Ok(DecodedMesh {
        positions,
        normals,
        tangents,
        bitangents,
        texcoords,
        secondary_texcoords,
        colors,
        indices: all_indices,
    })
}

fn unpack_unit_vector(raw_x: u16, raw_y: u16) -> [f32; 3] {
    let mut y = raw_y as i32;
    let z_negative = y >= 32768;
    if z_negative {
        y -= 32768;
    }

    let x = raw_x as f32 / 32767.4 * 2.0 - 1.0;
    let y = y as f32 / 32767.4 * 2.0 - 1.0;
    let z_sq = (1.0 - (x * x + y * y)).max(0.0);
    let mut z = z_sq.sqrt();
    if z_negative {
        z = -z;
    }

    [x, y, z]
}
