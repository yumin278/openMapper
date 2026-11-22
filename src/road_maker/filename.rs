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

    pub fn rel(&self) -> &str {
        &self.rel
    }

    pub(super) fn source(&self) -> &Path {
        &self.source
    }

    fn owns(&self, path: &Path) -> bool {
        if !std::fs::read(path.with_extension("road.json")).is_ok_and(|bytes| bytes == self.metadata) {
            return false;
        }
        // An imported draft has metadata, but no SLI until the user saves it.
        if !path.exists() { return self.source == path; }
        let (Ok(source), Ok(path)) = (self.source.canonicalize(), path.canonicalize()) else { return false };
        source == path
    }
}

impl RoadSpec {
    pub fn maker_filename(&self, edit: Option<&MakerTarget>) -> String {
        edit.map(|target| target.rel.clone()).unwrap_or_else(|| format!("Splines\\openMapper\\{}.sli", self.file_stem()))
    }

    pub fn maker_name_changed(&self, edit: Option<&MakerTarget>) -> bool {
        edit.is_some_and(|target| !self.maker_filename(None).eq_ignore_ascii_case(target.rel()))
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
        if (path.try_exists()? || path.with_extension("road.json").try_exists()?)
            && !edit.is_some_and(|target| target.owns(&path))
        {
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
    fn json_only_import_can_be_saved_once_without_clobbering_other_drafts() {
        let (root, template) = fixture("json-only");
        let mut draft = RoadSpec {
            name: "Imported".into(),
            imported_sli: true,
            advanced: true,
            pocket: 0.0,
            marked: false,
            hand: None,
            roadway_tex: template.roadway_tex,
            ..Default::default()
        };
        draft.rebuild_strips(false);
        let rel = draft.maker_filename(None);
        let sli = own_path(&root, &rel);
        std::fs::create_dir_all(sli.parent().unwrap()).unwrap();
        let json = sli.with_extension("road.json");
        let original = serde_json::to_vec_pretty(&draft).unwrap();
        std::fs::write(&json, &original).unwrap();

        assert!(!sli.exists(), "import must save JSON only");
        assert!(draft.write_in_maker(&root, false, None).is_err(), "another save must not overwrite this JSON draft");
        assert_eq!(std::fs::read(&json).unwrap(), original);

        let target = MakerTarget::open(rel.clone(), sli.clone()).unwrap();
        assert_eq!(draft.write_in_maker(&root, false, Some(&target)).unwrap(), rel);
        assert!(sli.is_file(), "the SLI appears only on explicit maker save");
        assert!(json.is_file());
        println!("Before maker save: {}\\n{}\\nAfter maker save: {}\\n{}", rel,
            String::from_utf8_lossy(&original), sli.display(),
            String::from_utf8_lossy(&std::fs::read(&json).unwrap()));
        std::fs::remove_dir_all(root).unwrap();
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

    #[test]
    fn renamed_road_can_be_saved_as_new_without_changing_the_original() {
        let (root, template) = fixture("save-as");
        let original = RoadSpec { name: "Original".into(), ..template };
        let old_rel = original.write_in_maker(&root, false, None).unwrap();
        let old_path = own_path(&root, &old_rel);
        let target = MakerTarget::open(old_rel, old_path.clone()).unwrap();
        let before = [std::fs::read(&old_path).unwrap(), std::fs::read(old_path.with_extension("road.json")).unwrap()];
        let renamed = RoadSpec { name: "Renamed road".into(), lane_width: 4.0, ..original };
        let new_rel = renamed.write_in_maker_as_new(&root, false, &target).unwrap();
        let new_path = own_path(&root, &new_rel);
        assert_eq!(new_rel, "Splines\\openMapper\\Renamed_road.sli");
        assert!(new_path.is_file() && new_path.with_extension("road.json").is_file());
        assert_eq!(std::fs::read(&old_path).unwrap(), before[0]);
        assert_eq!(std::fs::read(old_path.with_extension("road.json")).unwrap(), before[1]);
        assert_eq!(RoadSpec::of(&new_path).unwrap().name, "Renamed road");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_as_copies_the_original_bay_pixels_sidecars_and_marks() {
        let (root, template) = fixture("save-as-assets");
        let original = RoadSpec { name: "Old bay".into(), ..template };
        let old_rel = original.write_in_maker(&root, false, None).unwrap();
        let old_path = own_path(&root, &old_rel);
        let old_bay = old_path.parent().unwrap().join("texture/Old_bay_bay.png");
        let pixels = image::RgbaImage::from_pixel(4, 4, image::Rgba([7, 31, 211, 255]));
        pixels.save(&old_bay).unwrap();
        std::fs::write(format!("{}.cfg", old_bay.display()), b"bay cfg").unwrap();
        std::fs::write(old_bay.with_extension("surf"), b"bay surf").unwrap();
        let old_marks = old_path.with_extension("marks.png");
        image::RgbaImage::from_pixel(HAND_W, HAND_H, image::Rgba([1, 2, 3, 4])).save(&old_marks).unwrap();
        let target = MakerTarget::open(old_rel, old_path).unwrap();
        let renamed = RoadSpec { name: "New bay".into(), ..original };
        let new_rel = renamed.write_in_maker_as_new(&root, false, &target).unwrap();
        let new_path = own_path(&root, &new_rel);
        let new_bay = new_path.parent().unwrap().join("texture/New_bay_bay.png");
        assert_eq!(image::open(&new_bay).unwrap().to_rgba8(), pixels);
        assert_eq!(std::fs::read(format!("{}.cfg", new_bay.display())).unwrap(), b"bay cfg");
        assert_eq!(std::fs::read(new_bay.with_extension("surf")).unwrap(), b"bay surf");
        assert_eq!(std::fs::read(new_path.with_extension("marks.png")).unwrap(), std::fs::read(old_marks).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn renamed_road_overwrite_and_unchanged_name_keep_the_existing_filename() {
        for (tag, name) in [("overwrite-renamed", "Renamed"), ("overwrite-same", "Original")] {
            let (root, template) = fixture(tag);
            let original = RoadSpec { name: "Original".into(), ..template };
            let old_rel = original.write_in_maker(&root, false, None).unwrap();
            let old_path = own_path(&root, &old_rel);
            let target = MakerTarget::open(old_rel.clone(), old_path.clone()).unwrap();
            let edited = RoadSpec { name: name.into(), lane_width: 4.25, ..original };
            assert_eq!(edited.write_in_maker(&root, false, Some(&target)).unwrap(), old_rel);
            assert_eq!(RoadSpec::of(&old_path).unwrap().lane_width, 4.25);
            assert!(!own_path(&root, &edited.maker_filename(None)).exists() || edited.maker_filename(None).eq_ignore_ascii_case(&old_rel));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn save_as_refuses_a_filename_owned_by_another_road_without_changes() {
        let (root, template) = fixture("save-as-conflict");
        let original = RoadSpec { name: "Original".into(), ..template.clone() };
        let occupied = RoadSpec { name: "Occupied".into(), lane_width: 3.75, ..template };
        let old_rel = original.write_in_maker(&root, false, None).unwrap();
        let occupied_rel = occupied.write_in_maker(&root, false, None).unwrap();
        let old_path = own_path(&root, &old_rel);
        let occupied_path = own_path(&root, &occupied_rel);
        let target = MakerTarget::open(old_rel, old_path.clone()).unwrap();
        let before = [std::fs::read(&old_path).unwrap(), std::fs::read(&occupied_path).unwrap()];
        let renamed = RoadSpec { name: "Occupied".into(), lane_width: 5.0, ..original };
        assert!(renamed.write_in_maker_as_new(&root, false, &target).is_err());
        assert_eq!(std::fs::read(old_path).unwrap(), before[0]);
        assert_eq!(std::fs::read(occupied_path).unwrap(), before[1]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
