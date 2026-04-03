use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::archive::Archive;
use crate::mesh::decode_mesh;
use crate::scene::Scene;

#[derive(Debug, Clone)]
pub struct ObjExportOptions {
    pub included_meshes: BTreeSet<usize>,
    pub include_textures: bool,
}

pub fn export_scene(
    archive: &Archive,
    scene: &Scene,
    input_path: &Path,
    output_dir: &Path,
    options: &ObjExportOptions,
    progress: &mut dyn FnMut(u8, &str),
) -> Result<()> {
    fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let scene_name = input_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("scene");
    let obj_path = output_dir.join(format!("{scene_name}.obj"));
    let mtl_path = output_dir.join(format!("{scene_name}.mtl"));

    let mut obj = String::new();
    let mut mtl = String::new();
    obj.push_str(&format!("mtllib {}.mtl\n", scene_name));

    let total_meshes = scene.meshes.len().max(1);
    let mut vertex_base = 1usize;
    let mut texcoord_base = 1usize;
    let mut normal_base = 1usize;

    for (mesh_index, mesh_desc) in scene.meshes.iter().enumerate() {
        if !options.included_meshes.contains(&mesh_index) {
            continue;
        }

        let entry = archive
            .get(&mesh_desc.file)
            .with_context(|| format!("missing mesh payload {}", mesh_desc.file))?;
        let decoded = decode_mesh(&entry.data, mesh_desc)
            .with_context(|| format!("failed to decode {}", mesh_desc.file))?;

        obj.push_str(&format!("\no {}\n", sanitize_name(&mesh_desc.name)));
        for position in &decoded.positions {
            let position = transform_position(mesh_desc.transform.as_ref(), *position);
            obj.push_str(&format!("v {} {} {}\n", position[0], position[1], position[2]));
        }
        for texcoord in &decoded.texcoords {
            obj.push_str(&format!("vt {} {}\n", texcoord[0], texcoord[1]));
        }
        for normal in &decoded.normals {
            let normal = transform_normal(mesh_desc.transform.as_ref(), *normal);
            obj.push_str(&format!("vn {} {} {}\n", normal[0], normal[1], normal[2]));
        }

        for submesh in &mesh_desc.sub_meshes {
            obj.push_str(&format!("usemtl {}\n", sanitize_name(&submesh.material)));
            if let Some(material) = scene.materials.iter().find(|m| m.name == submesh.material) {
                append_material(&mut mtl, material, archive, output_dir, options.include_textures)?;
            }
            let start = submesh.first_index;
            let end = submesh.first_index + submesh.index_count;
            for face in decoded.indices[start..end].chunks(3) {
                if face.len() < 3 {
                    continue;
                }
                let a = face[0] as usize + vertex_base;
                let b = face[1] as usize + vertex_base;
                let c = face[2] as usize + vertex_base;
                let at = face[0] as usize + texcoord_base;
                let bt = face[1] as usize + texcoord_base;
                let ct = face[2] as usize + texcoord_base;
                let an = face[0] as usize + normal_base;
                let bn = face[1] as usize + normal_base;
                let cn = face[2] as usize + normal_base;
                obj.push_str(&format!(
                    "f {}/{}/{} {}/{}/{} {}/{}/{}\n",
                    a, at, an, b, bt, bn, c, ct, cn
                ));
            }
        }

        vertex_base += decoded.positions.len();
        texcoord_base += decoded.texcoords.len();
        normal_base += decoded.normals.len();
        let percent = 10 + ((mesh_index + 1) * 85 / total_meshes);
        progress(percent as u8, "Writing OBJ");
    }

    fs::write(&obj_path, obj).with_context(|| format!("failed to write {}", obj_path.display()))?;
    fs::write(&mtl_path, mtl).with_context(|| format!("failed to write {}", mtl_path.display()))?;
    progress(100, "OBJ export complete");
    Ok(())
}

fn append_material(
    mtl: &mut String,
    material: &crate::scene::MaterialDesc,
    archive: &Archive,
    output_dir: &Path,
    include_textures: bool,
) -> Result<()> {
    let material_name = sanitize_name(&material.name);
    if mtl.contains(&format!("newmtl {material_name}\n")) {
        return Ok(());
    }

    mtl.push_str(&format!("\nnewmtl {}\n", material_name));
    mtl.push_str("Kd 1.0 1.0 1.0\n");
    if matches!(material.blend.as_deref(), Some("alpha") | Some("add")) {
        mtl.push_str("d 0.999\n");
    }
    if !include_textures {
        return Ok(());
    }

    if let Some(name) = copy_texture(archive, output_dir, &material.albedo_tex)? {
        mtl.push_str(&format!("map_Kd {}\n", name));
    }
    if let Some(name) = material.alpha_tex.as_ref().and_then(|name| copy_texture(archive, output_dir, name).ok()).flatten() {
        mtl.push_str(&format!("map_d {}\n", name));
    }
    if let Some(name) = material.normal_tex.as_ref().and_then(|name| copy_texture(archive, output_dir, name).ok()).flatten() {
        mtl.push_str(&format!("map_Bump {}\n", name));
        mtl.push_str(&format!("bump {}\n", name));
    }
    if let Some(name) = material.reflectivity_tex.as_ref().and_then(|name| copy_texture(archive, output_dir, name).ok()).flatten() {
        mtl.push_str(&format!("map_Ks {}\n", name));
    }
    Ok(())
}

fn copy_texture(archive: &Archive, output_dir: &Path, name: &str) -> Result<Option<String>> {
    let Some(entry) = archive.get(name) else {
        return Ok(None);
    };
    let file_name = archive_relative_output_path(&entry.name);
    let output_path = output_dir.join(&file_name);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if !output_path.exists() {
        fs::write(&output_path, &entry.data)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
    }
    Ok(Some(path_to_slash(file_name)))
}

fn transform_position(transform: Option<&[f32; 16]>, position: [f32; 3]) -> [f32; 3] {
    let Some(m) = transform else {
        return position;
    };
    [
        m[0] * position[0] + m[4] * position[1] + m[8] * position[2] + m[12],
        m[1] * position[0] + m[5] * position[1] + m[9] * position[2] + m[13],
        m[2] * position[0] + m[6] * position[1] + m[10] * position[2] + m[14],
    ]
}

fn transform_normal(transform: Option<&[f32; 16]>, normal: [f32; 3]) -> [f32; 3] {
    let Some(m) = transform else {
        return normal;
    };
    normalize([
        m[0] * normal[0] + m[4] * normal[1] + m[8] * normal[2],
        m[1] * normal[0] + m[5] * normal[1] + m[9] * normal[2],
        m[2] * normal[0] + m[6] * normal[1] + m[10] * normal[2],
    ])
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        vector
    } else {
        [vector[0] / length, vector[1] / length, vector[2] / length]
    }
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') { ch } else { '_' })
        .collect()
}

fn archive_relative_output_path(name: &str) -> PathBuf {
    let mut path = PathBuf::new();
    let mut had_component = false;
    for component in name.split(['/', '\\']) {
        if component.is_empty() || component == "." || component == ".." {
            continue;
        }
        path.push(sanitize_name(component));
        had_component = true;
    }
    if had_component {
        path
    } else {
        PathBuf::from("unnamed.bin")
    }
}

fn path_to_slash(path: PathBuf) -> String {
    path.to_string_lossy().replace('\\', "/")
}
