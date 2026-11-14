//! Objects with lamps made in one click, ready to edit further: a signal head of chosen lamps
//! on a pole, and a street lamp that glows and lights the ground at night. The generated head
//! links to a crossing's light program like OMSI 2's `Ampel_Kfz_1`. The stock head faces local
//! +y (O3D +z, along the object's heading), while the generated head faces local -y.

use glam::{Mat4, Vec3};
use openmapper_mesh::primitives::Primitive;
use openmapper_mesh::{Material, Mesh, Model, Part};

use crate::editable::{Lamp, Light, LightKind, PartVar, Signal, Source, When};

/// A shape built, turned and moved into place, all its faces of one material.
fn shape(p: Primitive, m: Mat4, material: u32, smooth: bool) -> Mesh {
    let mut mesh = p.build();
    mesh.transform(m);
    for f in &mut mesh.faces {
        f.material = material;
        f.smooth = smooth && p.round();
    }
    mesh
}

/// A disc facing -y (a lens) of radius `r` and thickness `t`, its front at `y`.
fn lens(r: f32, t: f32, at: Vec3, material: u32) -> Mesh {
    shape(Primitive::Cylinder { radius: r, height: t, segments: 24 }, Mat4::from_translation(at) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2), material, true)
}

/// A signal head of `lamps` (top to bottom) on a pole 3.2 m high.
pub fn signal_head(lamps: &[Lamp]) -> Source {
    let lamps: Vec<Lamp> = if lamps.is_empty() { vec![Lamp::Red, Lamp::Yellow, Lamp::Green] } else { lamps.to_vec() };
    let n = lamps.len() as f32;
    let pitch = 0.265;
    let pole_h = 3.2;
    let body_h = n * pitch + 0.08;
    let bottom = pole_h - 0.2;
    let top = bottom + body_h;
    let mut materials = vec![
        Material::colored("Housing", [0.13, 0.14, 0.13, 1.0]),
        Material::colored("Pole", [0.45, 0.46, 0.47, 1.0]),
        Material::colored("Lens", [0.07, 0.07, 0.08, 1.0]),
    ];
    let mut body = Mesh::default();
    // the pole and the housing on it, a visor over each lamp
    body.append(&shape(Primitive::Cylinder { radius: 0.06, height: pole_h, segments: 16 }, Mat4::from_translation(Vec3::new(0.0, 0.12, 0.0)), 1, true), 0);
    body.append(&shape(Primitive::Cube { size: Vec3::new(0.32, 0.22, body_h) }, Mat4::from_translation(Vec3::new(0.0, 0.01, bottom)), 0, false), 0);
    let mut parts = Vec::new();
    let mut lights = Vec::new();
    let mut vars = Vec::new();
    for (i, lamp) in lamps.iter().enumerate() {
        let z = top - 0.04 - pitch * (i as f32 + 0.5);
        body.append(&lens(0.1, 0.02, Vec3::new(0.0, -0.1, z), 2), 0);
        body.append(&shape(Primitive::Cube { size: Vec3::new(0.26, 0.16, 0.012) }, Mat4::from_translation(Vec3::new(0.0, -0.18, z + 0.11)), 0, false), 0);
        // the lit lens: shown while the script has its variable on
        let c = lamp.rgb().map(|v| v as f32 / 255.0);
        materials.push(Material { name: format!("{} lit", lamp.var()), color: [c[0], c[1], c[2], 1.0], emissive: c, ..Default::default() });
        let mat = (materials.len() - 1) as u32;
        let name = format!("{}Lit", lamp.var());
        parts.push(Part::new(&name, lens(0.098, 0.022, Vec3::new(0.0, -0.104, z), mat)));
        vars.push(PartVar { part: name, var: lamp.var().into(), value: 1.0 });
        lights.push(Light { kind: LightKind::Glow, pos: Vec3::new(0.0, -0.13, z), dir: Vec3::Y, color: lamp.rgb(), size: 0.3, range: 0.0, cone: Some((20.0, 50.0)), when: When::Var(lamp.var().into()), star: true, brightness: 1.0 });
    }
    parts.insert(0, Part::new("Body", body));
    Source {
        name: "Signal head".into(),
        model: Model { parts, materials },
        lights,
        signal: Some(Signal { lamps }),
        part_vars: vars,
        ..Default::default()
    }
}

/// A street lamp: a pole 6 m high with an arm over the road (toward -y) and a head that
/// glows at night and lights the ground under it.
pub fn street_lamp() -> Source {
    let materials = vec![
        Material::colored("Pole", [0.42, 0.44, 0.45, 1.0]),
        Material { name: "Glass".into(), color: [1.0, 0.86, 0.6, 1.0], emissive: [0.0; 3], ..Default::default() },
        Material { name: "Glass lit".into(), color: [1.0, 0.86, 0.6, 1.0], emissive: [1.0, 0.82, 0.55], ..Default::default() },
    ];
    let mut body = Mesh::default();
    body.append(&shape(Primitive::Cone { radius_bottom: 0.11, radius_top: 0.06, height: 6.0, segments: 16 }, Mat4::IDENTITY, 0, true), 0);
    body.append(&shape(Primitive::Cylinder { radius: 0.04, height: 1.5, segments: 12 }, Mat4::from_translation(Vec3::new(0.0, 0.0, 5.9)) * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2), 0, true), 0);
    body.append(&shape(Primitive::Cube { size: Vec3::new(0.32, 0.6, 0.12) }, Mat4::from_translation(Vec3::new(0.0, -1.55, 5.85)), 0, false), 0);
    body.append(&shape(Primitive::Cube { size: Vec3::new(0.26, 0.5, 0.03) }, Mat4::from_translation(Vec3::new(0.0, -1.55, 5.83)), 1, false), 0);
    let glass = shape(Primitive::Cube { size: Vec3::new(0.26, 0.5, 0.03) }, Mat4::from_translation(Vec3::new(0.0, -1.55, 5.825)), 2, false);
    let at = Vec3::new(0.0, -1.55, 5.8);
    Source {
        name: "Street lamp".into(),
        model: Model { parts: vec![Part::new("Body", body), Part::new("GlassLit", glass)], materials },
        lights: vec![
            Light { kind: LightKind::Glow, pos: at, dir: Vec3::NEG_Z, color: [255, 214, 150], size: 1.6, range: 0.0, cone: None, when: When::Night, star: false, brightness: 0.9 },
            Light { kind: LightKind::Ground, pos: at, dir: Vec3::NEG_Z, color: [255, 214, 150], size: 0.0, range: 9.0, cone: None, when: When::Night, star: false, brightness: 1.0 },
        ],
        part_vars: vec![PartVar { part: "GlassLit".into(), var: "NightlightA".into(), value: 1.0 }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signal_head_is_an_object_openomsi_reads_as_one() {
        let dir = std::env::temp_dir().join(format!("om-signal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let roots = crate::install::Roots { omsi: dir.join("omsi"), content: dir.join("content") };
        let src = signal_head(&[Lamp::Red, Lamp::Yellow, Lamp::Green]);
        let rel = crate::editable::new_rel(&roots, "M", "SignalHead");
        crate::editable::save(&roots, &rel, &src).unwrap();
        let path = omsi_cfg::resolve_path(&roots.content, &rel);
        let sco = omsi_scenery::sco::SceneryObject::load(&path).unwrap();
        assert!(sco.is_traffic_light);
        assert!(sco.traffic_lights.is_empty());
        // the body and a lit lens for each lamp, each lens shown by its variable
        assert_eq!(sco.model.meshes.len(), 4);
        assert_eq!(sco.model.meshes[1].visible.as_ref().map(|v| v.0.as_str()), Some("Red"));
        assert_eq!(sco.model.meshes[0].light_enh_2.len(), 3);
        assert!(path.parent().unwrap().join("script").join("signal.osc").is_file());
        let lamp = street_lamp();
        let rel2 = crate::editable::new_rel(&roots, "M", "Lamp");
        crate::editable::save(&roots, &rel2, &lamp).unwrap();
        let sco2 = omsi_scenery::sco::SceneryObject::load(&omsi_cfg::resolve_path(&roots.content, &rel2)).unwrap();
        assert_eq!(sco2.model.meshes.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
