use super::*;
use crate::doc::safe::own_path;
use crate::i18n::tr;

/// The road and the version of its metadata opened in the maker.
pub struct MakerTarget {
    rel: String,
    source: PathBuf,
    metadata: Vec<u8>,
}

impl MakerTarget {
    pub fn open(rel: String, source: PathBuf) -> Option<Self> {
        let metadata = std::fs::read(source.with_extension("road.json")).ok()?;
        Some(Self { rel, source, metadata })
    }

    pub(super) fn rel(&self) -> &str {
        &self.rel
    }

    fn owns(&self, path: &Path) -> bool {
        let (Ok(source), Ok(path)) = (self.source.canonicalize(), path.canonicalize()) else { return false };
        source == path && std::fs::read(path.with_extension("road.json")).is_ok_and(|bytes| bytes == self.metadata)
    }
}

impl RoadSpec {
    pub fn maker_filename(&self, edit: Option<&MakerTarget>) -> String {
        edit.map(|target| target.rel.clone()).unwrap_or_else(|| format!("Splines\\openMapper\\{}.sli", self.file_stem()))
    }

    pub fn maker_filename_warning(&self, edit: Option<&MakerTarget>) -> Option<String> {
        let (stem, replaced) = crate::omsi_name::ascii_stem_with_replacements(&self.name);
        (stem.is_empty() || replaced).then(|| {
            tr("Some characters in the road name cannot be used. File: {file}", "Некоторые символы в названии дороги недопустимы. Файл: {file}")
                .replace("{file}", &self.maker_filename(edit))
        })
    }

    pub(super) fn check_maker_target(&self, content: &Path, edit: Option<&MakerTarget>) -> anyhow::Result<()> {
        let rel = self.maker_filename(edit);
        let path = own_path(content, &rel);
        if path.try_exists()? && !edit.is_some_and(|target| target.owns(&path)) {
            return Err(conflict(&rel));
        }

        Ok(())
    }
}

fn conflict(file: &str) -> anyhow::Error {
    anyhow::Error::msg(tr("Another road already uses this file: {file}", "Этот файл уже используется другой дорогой: {file}").replace("{file}", file))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> (PathBuf, RoadSpec) {
        let root = std::env::temp_dir().join(format!("om-maker-filename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let tex = root.join("asphalt.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([80, 80, 80, 255])).save(&tex).unwrap();
        (root, RoadSpec { roadway_tex: tex, pocket: 3.0, marked: true, ..Default::default() })
    }

    #[test]
    fn warnings_describe_the_actual_filename_and_only_underscore_replacements() {
        for (name, stem, warn) in [
            ("道路", "road", true), ("A_路B", "A_B", true), ("路A", "A", true),
            ("Дорога", "Doroga", false), ("Main-2", "Main-2", false), ("Main road", "Main_road", true),
        ] {
            let s = RoadSpec { name: name.into(), ..Default::default() };
            let warning = s.maker_filename_warning(None);
            let file = s.maker_filename(None);
            println!("Input {name:?}, saved filename {file:?}, warning {warning:?}");
            assert_eq!(file, format!("Splines\\openMapper\\{stem}.sli"));
            assert_eq!(warning.as_ref().is_some_and(|message| message.contains(&file)), warn);
        }
    }

    #[test]
    fn existing_splines_and_their_metadata_survive_collisions() {
        let (root, template) = fixture("collision");
        for (first, second) in [("中文", "道路"), ("Дорога", "Doroga"), ("With-dash", "with-dash")] {
            let road = RoadSpec { name: first.into(), ..template.clone() };
            let rel = road.write_in_maker(&root, false, None).unwrap();
            let path = own_path(&root, &rel);
            let json = path.with_extension("road.json");
            let before = [std::fs::read(&path).unwrap(), std::fs::read(&json).unwrap()];
            let other = RoadSpec { name: second.into(), lane_width: 4.0, ..road };
            let error = other.write_in_maker(&root, false, None).unwrap_err();
            let after = [std::fs::read(&path).unwrap(), std::fs::read(&json).unwrap()];
            for (label, files) in [("Before", &before), ("After", &after)] {
                println!("{label} {first:?} vs {second:?} at {rel}: {error}\n{}\n{}",
                    String::from_utf8_lossy(&files[0]), String::from_utf8_lossy(&files[1]));
            }
            assert_eq!(after, before);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editing_legacy_roads_uses_the_original_spline_and_new_texture_stem() {
        let (root, template) = fixture("edit");
        for (old_stem, name, normalized) in [
            ("中文", "中文", "road"), ("Дорога", "Дорога", "Doroga"), ("Main.2", "Main.2", "Main_2"),
        ] {
            let rel = format!("Splines\\openMapper\\{old_stem}.sli");
            let road = RoadSpec { name: name.into(), ..template.clone() };
            let path = own_path(&root, &rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let before = road.sli(false);
            std::fs::write(&path, &before).unwrap();
            std::fs::write(path.with_extension("road.json"), serde_json::to_vec(&road).unwrap()).unwrap();
            let target = MakerTarget::open(rel.clone(), path.clone()).unwrap();
            let mut edited = RoadSpec::of(&path).unwrap();
            edited.lane_width = 4.0;
            edited.rebuild_strips(false);
            assert_eq!(edited.write_in_maker(&root, false, Some(&target)).unwrap(), rel);
            let after = omsi_cfg::vfs::read_text(&path).unwrap();
            println!("Original: {rel}\n{before}\nNormalized: {normalized}.sli\nSaved: {rel}\n{after}");
            assert!(!path.with_file_name(format!("{normalized}.sli")).exists());
            assert_ne!(before, after.replace("\r\n", "\n"));
            assert_eq!(RoadSpec::of(&path).unwrap().lane_width, 4.0);
            for tex in [format!("{normalized}_lines.bmp"), format!("{normalized}_bay.png")] {
                assert!(after.contains(&tex));
                assert!(path.parent().unwrap().join("texture").join(tex).exists());
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
