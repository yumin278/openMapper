//! Renaming an object: the `[friendlyname]` of its `.sco`, the name the catalogue and OMSI 2
//! show it by. The file keeps every other line as it is; only the name's line is written,
//! encoded the way the file was (see `crossing_edit::encode_like`). An openMapper object's
//! own source (`<name>.openmapper.json`) says the name too, so it is changed with it, without
//! writing the object again (its meshes and their modifiers stay as they are).

use std::path::Path;

use crate::i18n::tr;
use crate::install::Roots;

/// The name as it is written: one line, not empty, not a keyword.
fn clean(name: &str) -> anyhow::Result<&str> {
    if name.contains('\r') || name.contains('\n') {
        anyhow::bail!("{}", tr("A name cannot have a line break", "Имя не может содержать перевод строки"));
    }
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("{}", tr("A name cannot be empty", "Имя не может быть пустым"));
    }
    // (a line that starts with `[` would be read as a keyword, the name would be lost)
    if name.starts_with('[') {
        anyhow::bail!("{}", tr("A name cannot begin with \"[\"", "Имя не может начинаться с «[»"));
    }
    Ok(name)
}

/// The file's text with its name set to `name`: its `[friendlyname]` block's value line, or
/// the block at the top of a file that has none. Every other line is left as it was.
fn renamed_text(text: &str, name: &str) -> String {
    let mut lines = omsi_cfg::split_lines(text);
    let at = lines.iter().position(|l| omsi_cfg::keyword_of(l).is_some_and(|k| k.eq_ignore_ascii_case("friendlyname")));
    match at {
        // (the value line: the line after the keyword, unless the block has none)
        Some(i) if i + 1 < lines.len() && omsi_cfg::keyword_of(&lines[i + 1]).is_none() => lines[i + 1] = name.to_string(),
        Some(i) => lines.insert(i + 1, name.to_string()),
        None => {
            lines.insert(0, String::new());
            lines.insert(0, name.to_string());
            lines.insert(0, "[friendlyname]".to_string());
        }
    }
    let mut out = String::new();
    for l in &lines {
        out.push_str(l);
        out.push('\n');
    }
    if text.contains("\r\n") {
        out = out.replace('\n', "\r\n");
    }
    out
}

/// What was written reads back as the same object, with the new name.
fn verify(trial: &Path, before: &omsi_scenery::SceneryObject, name: &str) -> anyhow::Result<()> {
    let sco = omsi_scenery::SceneryObject::load(trial).map_err(|e| anyhow::anyhow!("{e}"))?;
    if sco.friendly_name.trim() != name || sco.paths.len() != before.paths.len() || sco.model.meshes.len() != before.model.meshes.len() {
        anyhow::bail!("{}", tr("what was written does not read back the same", "записанное читается не так, как записано"));
    }
    Ok(())
}

/// Set the name of the object `rel` shows in the catalogue and in OMSI 2. False when the
/// file says that name already (nothing was written).
///
/// The file written is the one the catalogue shows: the content folder's copy when there is
/// one, the installation's own otherwise (as `crossing_edit::find` decides).
pub fn set_friendly_name(roots: &Roots, rel: &str, name: &str) -> anyhow::Result<bool> {
    let name = clean(name)?;
    let abs = crate::crossing_edit::find(roots, rel).ok_or_else(|| anyhow::anyhow!("{rel}: {}", tr("file not found", "файл не найден")))?;
    let bytes = std::fs::read(&abs)?;
    let text = omsi_cfg::decode_text(&bytes);
    let new = renamed_text(&text, name);
    if crate::crossing_edit::encode_like(&new, &bytes) == bytes {
        return Ok(false);
    }
    let before = omsi_scenery::SceneryObject::load(&abs).map_err(|e| anyhow::anyhow!("{}: {e}", tr("the object cannot be read, so it is left as it is", "объект не читается, поэтому оставлен как есть")))?;
    let bak = crate::crossing_edit::bak_path(&abs);
    let fresh_bak = !bak.exists();
    let root = if abs.starts_with(&roots.content) { "content" } else { "installation" };
    log::info!("rename {rel}: {} ({root}) {:?} -> {name:?}, backup: {fresh_bak}", abs.display(), before.friendly_name);

    // An openMapper object's source says the name too: read and prepared before anything is
    // written, so a source that cannot be read leaves the `.sco` as it was (a source naming
    // another name would put it back on the object's next save).
    let mut source: Option<(std::path::PathBuf, Vec<u8>)> = None;
    let mut json: Option<String> = None;
    if crate::editable::is_editable(roots, rel) {
        let path = crate::editable::source_path(roots, rel);
        let old = std::fs::read(&path).unwrap_or_default();
        let mut src = crate::editable::load(roots, rel).ok_or_else(|| anyhow::anyhow!("{}", tr("its openMapper source cannot be read, so the name would come back on its next save", "его исходник openMapper не читается, поэтому имя вернулось бы при следующем сохранении")))?;
        src.name = name.to_string();
        json = Some(serde_json::to_string_pretty(&src)?);
        source = Some((path, old));
    }

    // (checked beside the file first: a name that does not read back never touches what is there)
    let trial = abs.with_extension("crossing-check.sco");
    let checked = crate::crossing_edit::write(&trial, &new, &bytes).and_then(|_| verify(&trial, &before, name));
    if let Err(e) = checked {
        let _ = std::fs::remove_file(&trial);
        log::warn!("rename {rel}: {e:#}");
        return Err(e);
    }

    // (the source first: a failure here still leaves the `.sco` as it was)
    if let (Some((path, _)), Some(json)) = (&source, &json) {
        if let Err(e) = crate::doc::safe::write_atomic(path, json.as_bytes()) {
            let _ = std::fs::remove_file(&trial);
            let msg = format!("{}: {e}", tr("its openMapper source cannot be written", "его исходник openMapper не записывается"));
            log::warn!("rename {rel}: {msg}");
            return Err(anyhow::anyhow!("{msg}"));
        }
    }
    let put_back = |source: &Option<(std::path::PathBuf, Vec<u8>)>| -> Option<String> {
        let (path, old) = source.as_ref()?;
        crate::doc::safe::write_atomic(path, old).err().map(|e| format!("{}: {e}", tr("and its openMapper source could not be put back", "и его исходник openMapper не удалось вернуть")))
    };

    // (the file as it stands becomes the backup, unless there is one: that holds the file
    // from before the first rename, and a second one must not put the first one's text there)
    if fresh_bak {
        if let Err(e) = std::fs::copy(&abs, &bak) {
            let _ = std::fs::remove_file(&trial);
            let mut msg = format!("{}: {e}", tr("the name could not be saved", "название не удалось сохранить"));
            if let Some(back) = put_back(&source) {
                msg.push_str("; ");
                msg.push_str(&back);
            }
            log::warn!("rename {rel}: {msg}");
            return Err(anyhow::anyhow!("{msg}"));
        }
        crate::crossing_edit::baks::note(roots, &bak);
    }
    if let Err(e) = std::fs::rename(&trial, &abs) {
        let _ = std::fs::remove_file(&trial);
        let mut msg = format!("{}: {e}", tr("the name could not be saved", "название не удалось сохранить"));
        if let Some(back) = put_back(&source) {
            msg.push_str("; ");
            msg.push_str(&back);
        }
        log::warn!("rename {rel}: {msg}");
        return Err(anyhow::anyhow!("{msg}"));
    }
    crate::hot::own_change();
    log::info!("rename {rel}: {} ({root}) is now {name:?}", abs.display());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Asset, Category, Kind, Library};
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn roots(tag: &str) -> (PathBuf, Roots) {
        let dir = std::env::temp_dir().join(format!("om-rename-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("content")).unwrap();
        std::fs::create_dir_all(dir.join("omsi")).unwrap();
        (dir.clone(), Roots { omsi: dir.join("omsi"), content: dir.join("content") })
    }

    fn sco(name: &str) -> Vec<u8> {
        format!("[friendlyname]\r\n{name}\r\n\r\n[groups]\r\n1\r\nTest\r\n\r\n[mesh]\r\ntest.o3d\r\n\r\n").into_bytes()
    }

    /// A Windows-1252 `.sco` (CR LF) whose name is `Grüße`: `0xFC` and `0xDF` are not UTF-8.
    fn cp1252_sco() -> Vec<u8> {
        let mut b = b"[friendlyname]\r\nGr".to_vec();
        b.extend_from_slice(&[0xFC, 0xDF, b'e']);
        b.extend_from_slice(b"\r\n\r\n[groups]\r\n1\r\nTest\r\n\r\n[mesh]\r\ntest.o3d\r\n\r\n");
        b
    }

    fn write(roots: &Roots, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = omsi_cfg::resolve_path(&roots.content, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn write_in(dir: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = omsi_cfg::resolve_path(dir, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    const REL: &str = "Sceneryobjects\\Test\\Thing.sco";

    #[test]
    fn rename_crlf_cp1252_preserves_other_bytes_and_backup() {
        let (dir, roots) = roots("cp1252");
        let original = cp1252_sco();
        let p = write(&roots, REL, &original);
        assert!(set_friendly_name(&roots, REL, "New Name").unwrap());
        let now = std::fs::read(&p).unwrap();
        let want = b"[friendlyname]\r\nNew Name\r\n\r\n[groups]\r\n1\r\nTest\r\n\r\n[mesh]\r\ntest.o3d\r\n\r\n".to_vec();
        assert_eq!(now, want, "only the name's line changes");
        assert_eq!(std::fs::read(crate::crossing_edit::bak_path(&p)).unwrap(), original, "the backup is the file from before");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_inserts_missing_friendlyname_at_start() {
        let (dir, roots) = roots("missing");
        let p = write(&roots, REL, b"[groups]\r\n1\r\nTest\r\n\r\n[mesh]\r\ntest.o3d\r\n\r\n");
        assert!(set_friendly_name(&roots, REL, "Named").unwrap());
        let text = omsi_cfg::decode_text(&std::fs::read(&p).unwrap());
        assert!(text.starts_with("[friendlyname]\r\nNamed\r\n\r\n[groups]\r\n1\r\nTest\r\n"), "{text:?}");
        let sco = omsi_scenery::SceneryObject::load(&p).unwrap();
        assert_eq!(sco.friendly_name.trim(), "Named");
        assert_eq!(sco.model.meshes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_chinese_falls_back_to_utf8_bom() {
        let (dir, roots) = roots("chinese");
        let p = write(&roots, REL, &cp1252_sco());
        assert!(set_friendly_name(&roots, REL, "公車站").unwrap());
        let now = std::fs::read(&p).unwrap();
        assert_eq!(&now[..3], &[0xEF, 0xBB, 0xBF], "a name Windows-1252 cannot hold is written as UTF-8");
        let text = omsi_cfg::decode_text(&now);
        assert!(text.contains("公車站"), "{text:?}");
        let sco = omsi_scenery::SceneryObject::load(&p).unwrap();
        assert_eq!(sco.friendly_name.trim(), "公車站");
        assert_eq!(sco.model.meshes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_installation_only_in_place() {
        let (dir, roots) = roots("install");
        let p = write_in(&roots.omsi, REL, &sco("Old"));
        assert!(!omsi_cfg::resolve_path(&roots.content, REL).exists());
        assert!(set_friendly_name(&roots, REL, "New").unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), sco("New"), "the installation's file is the one the catalogue shows");
        assert_eq!(std::fs::read(crate::crossing_edit::bak_path(&p)).unwrap(), sco("Old"));
        assert!(!omsi_cfg::resolve_path(&roots.content, REL).exists(), "nothing is copied into the content folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_content_shadows_installation() {
        let (dir, roots) = roots("shadow");
        let p = write(&roots, REL, &sco("Old"));
        let theirs = write_in(&roots.omsi, REL, &sco("Theirs"));
        let before = std::fs::metadata(&theirs).unwrap().modified().unwrap();
        assert!(set_friendly_name(&roots, REL, "New").unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), sco("New"));
        assert_eq!(std::fs::read(&theirs).unwrap(), sco("Theirs"), "the installation's copy is untouched");
        assert_eq!(std::fs::metadata(&theirs).unwrap().modified().unwrap(), before);
        assert!(!crate::crossing_edit::bak_path(&theirs).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_rejects_invalid_names_without_writes() {
        let (dir, roots) = roots("invalid");
        let original = sco("Old");
        let p = write(&roots, REL, &original);
        for bad in ["", "   ", "a\nb", "a\rb", "Name\n", " [keyword]", "[groups]"] {
            assert!(set_friendly_name(&roots, REL, bad).is_err(), "{bad:?} is no name");
            assert_eq!(std::fs::read(&p).unwrap(), original, "{bad:?} left the file as it was");
            assert!(!crate::crossing_edit::bak_path(&p).exists(), "{bad:?} made no backup");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_same_name_is_noop_and_keeps_first_backup() {
        let (dir, roots) = roots("same");
        let original = sco("Old");
        let p = write(&roots, REL, &original);
        assert!(set_friendly_name(&roots, REL, "New").unwrap());
        let once = std::fs::read(&p).unwrap();
        assert!(!set_friendly_name(&roots, REL, "New").unwrap(), "the file says that name already");
        assert_eq!(std::fs::read(&p).unwrap(), once);
        assert_eq!(std::fs::read(crate::crossing_edit::bak_path(&p)).unwrap(), original, "the first backup stays the first one");
        assert!(!set_friendly_name(&roots, REL, "  New  ").unwrap(), "the name is trimmed before it is compared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_leaves_a_keyword_omsi_does_not_know_alone() {
        // (OMSI compares keyword lines exactly: `[FriendlyName]` is free text to it, so the
        // block the game reads is the one written at the top)
        let (dir, roots) = roots("case");
        let p = write(&roots, REL, b"[FriendlyName]\r\nOld\r\n\r\n[mesh]\r\ntest.o3d\r\n\r\n");
        assert!(set_friendly_name(&roots, REL, "New").unwrap());
        let text = omsi_cfg::decode_text(&std::fs::read(&p).unwrap());
        assert!(text.starts_with("[friendlyname]\r\nNew\r\n\r\n[FriendlyName]\r\nOld\r\n"), "{text:?}");
        assert_eq!(omsi_scenery::SceneryObject::load(&p).unwrap().friendly_name.trim(), "New");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_editable_updates_json_without_rewriting_meshes() {
        let (dir, roots) = roots("editable");
        let p = write(&roots, REL, &sco("Old"));
        let src = crate::editable::Source { name: "Old".into(), ..Default::default() };
        let json = crate::editable::source_path(&roots, REL);
        std::fs::write(&json, serde_json::to_string_pretty(&src).unwrap()).unwrap();
        let mesh = p.parent().unwrap().join("model").join("Thing_1.x");
        std::fs::create_dir_all(mesh.parent().unwrap()).unwrap();
        std::fs::write(&mesh, b"mesh bytes").unwrap();
        let before: SystemTime = std::fs::metadata(&mesh).unwrap().modified().unwrap();
        assert!(crate::editable::is_editable(&roots, REL));

        assert!(set_friendly_name(&roots, REL, "New").unwrap());

        let back = crate::editable::load(&roots, REL).unwrap();
        assert_eq!(back.name, "New", "the source says the new name, so the next save keeps it");
        assert_eq!(std::fs::read(&mesh).unwrap(), b"mesh bytes", "the object's meshes are not written again");
        assert_eq!(std::fs::metadata(&mesh).unwrap().modified().unwrap(), before);
        assert_eq!(std::fs::read(&p).unwrap(), sco("New"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_leaves_a_file_alone_whose_source_is_broken() {
        let (dir, roots) = roots("broken");
        let original = sco("Old");
        let p = write(&roots, REL, &original);
        std::fs::write(crate::editable::source_path(&roots, REL), b"{ not json").unwrap();
        assert!(set_friendly_name(&roots, REL, "New").is_err());
        assert_eq!(std::fs::read(&p).unwrap(), original, "the object is left as it was");
        assert!(!crate::crossing_edit::bak_path(&p).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_fails_without_touching_anything_when_the_file_is_not_there() {
        let (dir, roots) = roots("absent");
        assert!(set_friendly_name(&roots, REL, "New").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn asset(rel: &str, name: &str, haystack: &str) -> Asset {
        Asset {
            kind: Kind::Object,
            rel: rel.to_string(),
            name: name.to_string(),
            dir: "Test".into(),
            category: Category::Objects,
            haystack: haystack.to_string(),
            from_content: false,
            is_bus_stop: false,
            is_entry_point: false,
            is_car_park: false,
            abs_height: false,
            lamp: false,
            light_head: false,
            light_program: false,
            helpers: Vec::new(),
            paths: 0,
            half_width: 0.0,
            lanes: (0, 0, 0),
        }
    }

    #[test]
    fn library_rename_keeps_order_and_the_rest_of_the_search_text() {
        let mut lib = Library::default();
        lib.assets.push(asset("A.sco", "Lamp", "lamp a.sco 1 street lampen"));
        lib.assets.push(asset("B.sco", "Bench", "bench b.sco 1 street furniture"));
        lib.assets.push(asset("C.sco", "Tree", "tree c.sco 1 nature"));
        lib.reindex_for_tests();
        assert_eq!(lib.rev, 0);

        lib.rename("A.sco", "Street Lamp");

        assert_eq!(lib.rev, 1);
        assert_eq!(lib.assets[0].name, "Street Lamp");
        assert_eq!(lib.assets[0].haystack, "street lamp a.sco 1 street lampen", "the rest of the search text stays");
        assert_eq!(lib.assets[1].name, "Bench");
        assert_eq!(lib.assets[2].name, "Tree", "nothing is sorted again: the grid's cache holds positions");
        assert_eq!(lib.find("a.sco").unwrap().name, "Street Lamp");
        assert!(lib.filter(Kind::Object, None, "street lamp").any(|a| a.rel == "A.sco"));

        // a search text that does not begin with the name gets the new one in front of it
        lib.assets[1].haystack = "furniture b.sco".into();
        lib.rename("B.sco", "Park Bench");
        assert_eq!(lib.assets[1].haystack, "park bench furniture b.sco");
        assert_eq!(lib.rev, 2);

        // an asset the catalogue does not know is left alone
        lib.rename("D.sco", "Nothing");
        assert_eq!(lib.rev, 2);
    }
}
