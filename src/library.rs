//! The asset library: every scenery object and spline type of the installation and the
//! content folder, sorted into a few plain categories a beginner can find things in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use crate::i18n::tr;
use crate::install::Roots;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Object,
    Spline,
}

/// What an asset is, read from its own file (its OMSI tags and traffic paths), never from
/// its name: how it is organised is left to its folder (see `Asset::dir`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    // objects
    /// Anything to look at (buildings, plants, street furniture...: their folders say which).
    Objects,
    /// `[tree]`: a tree card of a texture.
    Trees,
    /// `[busstop]` and `[entrypoint]`.
    Stops,
    /// Has `[splinehelper]`s or traffic lights: a junction or crossing piece.
    Junctions,
    // splines
    /// Carries a street lane.
    Roads,
    /// Carries a rail lane.
    Tracks,
    /// Carries only footway lanes.
    Paths,
    /// Visible, no lanes: fences, walls, kerbs, embankments.
    Decoration,
    /// Not meant to be seen: AI-only splines and helpers (`[onlyeditor]`, no visible
    /// profile, no mesh), help arrows, parking spaces and sound sources.
    Technical,
}

impl Category {
    pub const OBJECTS: [Category; 5] = [Category::Objects, Category::Trees, Category::Stops, Category::Junctions, Category::Technical];
    pub const SPLINES: [Category; 5] = [Category::Roads, Category::Tracks, Category::Paths, Category::Decoration, Category::Technical];

    pub fn label(self) -> &'static str {
        match self {
            Category::Objects => tr("Objects", "Объекты"),
            Category::Trees => tr("Trees", "Деревья"),
            Category::Stops => tr("Stops and start points", "Остановки и точки старта"),
            Category::Junctions => tr("Junctions", "Перекрёстки"),
            Category::Roads => tr("Roads", "Дороги"),
            Category::Tracks => tr("Tracks", "Рельсы"),
            Category::Paths => tr("Footpaths", "Тротуары"),
            Category::Decoration => tr("Fences, walls, kerbs", "Заборы, стены, бордюры"),
            Category::Technical => tr("Technical (AI, helpers)", "Технические (ИИ, хелперы)"),
        }
    }

    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Category::Objects => Icon::Building,
            Category::Trees => Icon::Tree,
            Category::Stops => Icon::Bus,
            Category::Junctions => Icon::Junction,
            Category::Roads => Icon::Road,
            Category::Tracks => Icon::Tram,
            Category::Paths => Icon::Walk,
            Category::Decoration => Icon::Fence,
            Category::Technical => Icon::Construction,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Asset {
    pub kind: Kind,
    /// Path relative to its content root, with OMSI's backslashes (what a map file names).
    pub rel: String,
    pub name: String,
    /// Its folder below `Sceneryobjects` / `Splines` ("Streetobjects_MC\\Lampen"): the
    /// catalogue's tree.
    pub dir: String,
    pub category: Category,
    /// Lower-case text searched in.
    pub haystack: String,
    /// From the content folder (a mod or an import).
    pub from_content: bool,
    // objects
    pub is_bus_stop: bool,
    pub is_entry_point: bool,
    /// `[carpark_p]`: a place for a parked car.
    pub is_car_park: bool,
    pub abs_height: bool,
    /// Lights the ground round it at night (`[maplight]`: a street lamp).
    pub lamp: bool,
    /// A signal head (`[trafficlight]`, no program of its own): it lights only when linked to
    /// a crossing (`[varparent]`).
    pub light_head: bool,
    /// Runs a traffic light program (`[traffic_light]`s): a crossing signal heads link to.
    pub light_program: bool,
    /// Has `[splinehelper]` points: a junction or crossing piece that splines snap to.
    pub helpers: Vec<omsi_scenery::sco::SplineHelper>,
    /// How many `[path]`s it has: only an object with paths can be a crossing the editor
    /// works on, whatever else it holds (a signal head is in Junctions too, and has none).
    pub paths: usize,
    // splines
    /// Half the width of the drawn profile (m).
    pub half_width: f32,
    pub lanes: (u8, u8, u8),
}

/// The root a `.sco` found for a map's path actually lies under, with whether that root is the
/// content folder (else the installation). A lookup can answer with a file under another root
/// than the one asked - a copy under a higher-priority content root, or the installation for
/// what the content folder lacks - so the root a catalogue entry is named by is the one that
/// *holds* the file, the longest of these roots being the one (the content folder often sits
/// inside the installation). None when the file lies under neither.
fn owner_of<'a>(path: &Path, roots: &'a Roots) -> Option<(&'a Path, bool)> {
    let mut best: Option<(&'a Path, bool)> = None;
    for (root, from_content) in [(&roots.content, true), (&roots.omsi, false)] {
        if !path.starts_with(root) {
            continue;
        }
        // (roots can be nested - the content folder inside the installation, or the other way
        // round: the one that holds the file is the longer of the two)
        if best.is_none_or(|(found, _)| root.components().count() > found.components().count()) {
            best = Some((root.as_path(), from_content));
        }
    }
    best
}

/// Objects the editor wrote and the catalogue has not heard of yet: the content root they were
/// written under, and their path in a map's own spelling (`Sceneryobjects\...`). The root is
/// kept with each: a queue drained after another map (or another content folder) was opened
/// would otherwise read an object of the same name from the wrong root.
static WRITTEN: std::sync::Mutex<Vec<(std::path::PathBuf, String)>> = std::sync::Mutex::new(Vec::new());

/// Note an object the editor has just written under `root` (a junction, a bus bay's taper
/// plate, a plate of a drawn junction): [`Library::refresh_pending`] reads it into the
/// catalogue, since the hot reload takes the editor's own writes as already known.
pub(crate) fn note_written(root: &Path, rel: &str) {
    let mut queue = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    if !queue.iter().any(|(known_root, known)| known_root == root && known.eq_ignore_ascii_case(rel)) {
        queue.push((root.to_path_buf(), rel.to_string()));
    }
}

/// Whether anything was noted for any roots. The frame asks this before taking a copy of the
/// roots: most frames have nothing the editor wrote.
pub(crate) fn has_written() -> bool {
    !WRITTEN.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// The entries [`note_written`] left for `roots`, taken out of the queue; entries noted under
/// another content folder stay for the session that owns them.
fn take_written(roots: &Roots) -> Vec<String> {
    let mut queue = WRITTEN.lock().unwrap_or_else(|e| e.into_inner());
    let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *queue)
        .into_iter()
        .partition(|(root, _)| root == &roots.content || root == &roots.omsi);
    *queue = others;
    mine.into_iter().map(|(_, rel)| rel).collect()
}

/// Messages of the background scan.
pub enum ScanMsg {
    Found(Vec<Asset>),
    Done,
}

#[derive(Default)]
pub struct Library {
    pub assets: Vec<Asset>,
    pub scanning: bool,
    /// Goes up whenever an asset's own facts change without a scan (a rename): what the asset
    /// browser works out of the library is worked out again then.
    pub rev: u64,
    rx: Option<mpsc::Receiver<ScanMsg>>,
    /// Where each asset is in `assets`, by its lower-case path (`find` is asked many times
    /// a frame).
    index: HashMap<String, usize, std::hash::BuildHasherDefault<Fx>>,
}

/// The hash of the index: Fx's multiply and rotate, a good deal quicker than SipHash on a
/// path of fifty bytes asked for thousands of times a frame.
#[derive(Default)]
pub struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(K);
        }
        for &b in chunks.remainder() {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(K);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

impl Library {
    /// Scan both roots in the background.
    pub fn start_scan(&mut self, roots: &Roots) {
        let (tx, rx) = mpsc::channel();
        let dirs: Vec<(PathBuf, bool)> = vec![(roots.content.clone(), true), (roots.omsi.clone(), false)];
        std::thread::spawn(move || scan(dirs, tx));
        self.assets.clear();
        self.index.clear();
        self.rx = Some(rx);
        self.scanning = true;
    }

    /// Take what the scan found so far.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.rx {
            while let Ok(m) = rx.try_recv() {
                match m {
                    ScanMsg::Found(v) => {
                        for a in v {
                            // the content folder's copy hides the installation's
                            let key = a.rel.to_lowercase();
                            if !self.index.contains_key(&key) {
                                self.index.insert(key, self.assets.len());
                                self.assets.push(a);
                            }
                        }
                        changed = true;
                    }
                    ScanMsg::Done => {
                        self.scanning = false;
                        self.assets.sort_by_cached_key(|a| a.name.to_lowercase());
                        self.index = self.assets.iter().enumerate().map(|(i, a)| (a.rel.to_lowercase(), i)).collect();
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// The index rebuilt after a test pushed assets straight into `assets`.
    #[cfg(test)]
    pub fn reindex_for_tests(&mut self) {
        self.index = self.assets.iter().enumerate().map(|(i, a)| (a.rel.to_lowercase(), i)).collect();
    }

    /// An asset was renamed: its name, and the search text with it. Its place in `assets` is
    /// kept (the asset browser's cache holds positions): the list is not sorted again here.
    pub fn rename(&mut self, rel: &str, name: &str) {
        let Some(&i) = self.index.get(&rel.to_lowercase()) else { return };
        let Some(a) = self.assets.get_mut(i) else { return };
        let old = a.name.to_lowercase();
        let new = name.to_lowercase();
        // (`haystack` is the name, then the path, the groups and the folder: only the name's
        // part is swapped, the rest is kept as the scan worked it out)
        a.name = name.to_string();
        a.haystack = match a.haystack.strip_prefix(&old) {
            Some(rest) => format!("{new}{rest}"),
            None => format!("{new} {}", a.haystack),
        };
        self.rev += 1;
    }

    /// Refresh one saved crossing without rescanning or reordering the catalogue. True when the
    /// catalogue now holds it.
    pub(crate) fn refresh_crossing(&mut self, roots: &Roots, rel: &str) -> bool {
        // (the file is looked for in the spelling the map names it in: only the index key is
        // folded to lower case. Folding the path too makes every refresh a case-insensitive
        // directory lookup, and that lookup's cached listing can be older than the file - a
        // junction written into a folder made a moment ago would then not be found at all)
        let rel = rel.replace('/', "\\");
        let Some(path) = [&roots.content, &roots.omsi]
            .into_iter()
            .map(|root| omsi_cfg::resolve_path(root, &rel))
            .find(|path| omsi_cfg::vfs::is_file(path))
        else {
            log::warn!("catalogue: {rel}: not found in the content folder or the installation; not read again");
            return false;
        };
        // (the root that answered is not always the root that was asked: the game's lookup
        // mirrors a path onto the other content roots, so the entry is named by the root that
        // actually holds the file. Taking the asked root would leave `describe` with a path it
        // cannot strip a prefix from, and the object would silently never reach the catalogue)
        let Some((root, from_content)) = owner_of(&path, roots) else {
            log::warn!(
                "catalogue: {rel}: {} is under neither the content folder ({}) nor the installation ({}); not read again",
                path.display(),
                roots.content.display(),
                roots.omsi.display()
            );
            return false;
        };
        let Some(asset) = describe(root, &path, Kind::Object, from_content) else {
            log::warn!("catalogue: {rel}: {} cannot be read as an object; not read again", path.display());
            return false;
        };
        log::info!("catalogue: {} read again from the {}", asset.rel, if from_content { "content folder" } else { "installation" });
        let key = rel.to_lowercase();
        if let Some(&index) = self.index.get(&key) {
            self.assets[index] = asset;
        } else {
            let index = self.assets.len();
            self.index.insert(asset.rel.to_lowercase(), index);
            self.assets.push(asset);
        }
        self.rev += 1;
        true
    }

    /// Read every object the editor wrote under these roots since the last call into the
    /// catalogue, without a rescan: the hot reload does not report the editor's own writes, and
    /// a full scan would sort the catalogue again under the browser's cache. Returns how many
    /// were actually read (each failure is logged by [`Self::refresh_crossing`]); what was
    /// written under another content folder stays queued for that one, and nothing is retried.
    pub(crate) fn refresh_pending(&mut self, roots: &Roots) -> usize {
        let written = take_written(roots);
        let total = written.len();
        let mut read = 0;
        for rel in &written {
            if self.refresh_crossing(roots, rel) {
                read += 1;
            }
        }
        if read < total {
            log::warn!("catalogue: {} of {total} object(s) the editor wrote could not be read again", total - read);
        }
        read
    }

    /// A file written while the editor runs (a road type of one's own, a made object) taken into
    /// the catalogue at once, replacing what it said of that file before.
    pub fn add_file(&mut self, root: &Path, path: &Path, kind: Kind) -> bool {
        let Some(a) = describe(root, path, kind, true) else { return false };
        let key = a.rel.to_lowercase();
        match self.index.get(&key) {
            Some(&i) => self.assets[i] = a,
            None => {
                self.index.insert(key, self.assets.len());
                self.assets.push(a);
            }
        }
        true
    }

    pub fn find(&self, rel: &str) -> Option<&Asset> {
        // (an ASCII path, nearly all of them, is lower-cased on the stack: no allocation)
        let mut buf = [0u8; 192];
        let i = if rel.is_ascii() && rel.len() <= buf.len() {
            let low = &mut buf[..rel.len()];
            low.copy_from_slice(rel.as_bytes());
            low.make_ascii_lowercase();
            self.index.get(std::str::from_utf8(low).unwrap_or(""))
        } else {
            self.index.get(&rel.to_lowercase())
        };
        i.and_then(|&i| self.assets.get(i))
    }

    /// Assets of a kind and category matching a search (words in any order).
    /// The junction pieces road type `road` can be joined to: those with a place made for it
    /// (`fits`), else, when none is, every piece that has places for roads at all.
    pub fn junction_pieces(&self, road: &str) -> (Vec<&Asset>, bool) {
        let norm = |p: &str| p.replace('/', "\\").to_lowercase();
        let road = norm(road);
        let pieces: Vec<&Asset> = self.assets.iter().filter(|a| a.kind == Kind::Object && !a.helpers.is_empty()).collect();
        let fitting: Vec<&Asset> = pieces.iter().copied().filter(|a| a.helpers.iter().any(|h| norm(&h.spline) == road)).collect();
        if fitting.is_empty() { (pieces, false) } else { (fitting, true) }
    }

    pub fn filter<'a>(&'a self, kind: Kind, cat: Option<Category>, search: &str) -> impl Iterator<Item = &'a Asset> + 'a + use<'a> {
        let words: Vec<String> = search.to_lowercase().split_whitespace().map(str::to_string).collect();
        self.assets
            .iter()
            .filter(move |a| a.kind == kind && cat.map(|c| c == a.category).unwrap_or(true) && words.iter().all(|w| a.haystack.contains(w)))
    }

    pub fn count(&self, kind: Kind, cat: Category) -> usize {
        self.assets.iter().filter(|a| a.kind == kind && a.category == cat).count()
    }
}

fn scan(dirs: Vec<(PathBuf, bool)>, tx: mpsc::Sender<ScanMsg>) {
    for (root, from_content) in dirs {
        for (sub, ext, kind) in [("Splines", "sli", Kind::Spline), ("Sceneryobjects", "sco", Kind::Object)] {
            let Some(base) = find_child(&root, sub) else { continue };
            let mut files = Vec::new();
            walk(&base, ext, &mut files);
            let mut batch = Vec::new();
            for f in files {
                if let Some(a) = describe(&root, &f, kind, from_content) {
                    batch.push(a);
                }
                if batch.len() >= 64 {
                    let _ = tx.send(ScanMsg::Found(std::mem::take(&mut batch)));
                }
            }
            let _ = tx.send(ScanMsg::Found(batch));
        }
    }
    let _ = tx.send(ScanMsg::Done);
}

/// A folder's child of that name whatever its case.
fn find_child(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_dir() && p.file_name().map(|f| f.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().map(|x| x.to_string_lossy().eq_ignore_ascii_case(ext)).unwrap_or(false) {
            out.push(p);
        }
    }
}

/// "str_2spur_11m_SeeburgerStr1" > "Str 2spur 11m SeeburgerStr1".
fn pretty(stem: &str) -> String {
    let s = stem.replace(['_', '-'], " ");
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

/// A catalogue of road splines of the given half widths, for tests.
#[cfg(test)]
pub fn roads_for_tests(list: &[(&str, f32)]) -> Library {
    let mut lib = Library::default();
    for (rel, half) in list {
        let a = Asset {
            kind: Kind::Spline,
            rel: rel.to_string(),
            name: rel.to_string(),
            dir: String::new(),
            category: Category::Roads,
            haystack: String::new(),
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
            half_width: *half,
            lanes: (0, 0, 0),
        };
        lib.assets.push(a);
    }
    lib.index = lib.assets.iter().enumerate().map(|(i, a)| (a.rel.to_lowercase(), i)).collect();
    lib
}

fn describe(root: &Path, path: &Path, kind: Kind, from_content: bool) -> Option<Asset> {
    let rel = path.strip_prefix(root).ok()?.to_string_lossy().replace('/', "\\");
    let stem = path.file_stem()?.to_string_lossy().to_string();
    let folder = path.parent().and_then(|p| p.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    let parts: Vec<&str> = rel.split('\\').collect();
    let dir = if parts.len() > 2 { parts[1..parts.len() - 1].join("\\") } else { String::new() };
    let mut a = Asset {
        kind,
        rel: rel.clone(),
        name: pretty(&stem),
        dir,
        category: Category::Objects,
        haystack: String::new(),
        from_content,
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
    };
    let mut groups = String::new();
    match kind {
        Kind::Object => {
            let sco = match omsi_scenery::SceneryObject::load(path) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("catalogue: {} cannot be read: {e}", path.display());
                    return None;
                }
            };
            if !sco.friendly_name.trim().is_empty() {
                a.name = sco.friendly_name.trim().to_string();
            }
            groups = sco.groups.join(" ").to_lowercase();
            a.is_bus_stop = sco.is_bus_stop;
            a.is_entry_point = sco.is_entry_point;
            a.is_car_park = sco.is_car_park;
            a.abs_height = sco.absolute_height();
            a.lamp = !sco.map_lights.is_empty();
            a.light_head = sco.is_traffic_light && sco.traffic_lights.is_empty();
            a.light_program = !sco.traffic_lights.is_empty();
            a.helpers = sco.spline_helpers.clone();
            a.paths = sco.paths.len();
            let has_mesh = !sco.model.meshes.is_empty() || sco.model_file.is_some() || sco.tree.is_some();
            a.category = if sco.is_bus_stop || sco.is_entry_point {
                Category::Stops
            } else if sco.tree.is_some() {
                // (before what only the editor shows: a tree's helper box is `[onlyeditor]`,
                // its tree is seen in the game)
                Category::Trees
            } else if sco.only_editor || sco.is_help_arrow || sco.is_car_park || !has_mesh {
                Category::Technical
            } else if !sco.spline_helpers.is_empty() || sco.is_traffic_light || !sco.traffic_lights.is_empty() {
                Category::Junctions
            } else {
                Category::Objects
            };
        }
        Kind::Spline => {
            let sli = match omsi_scenery::Spline::load(path) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("catalogue: {} cannot be read: {e}", path.display());
                    return None;
                }
            };
            let mut w = 0.0f32;
            for p in &sli.profiles {
                for q in &p.points {
                    w = w.max(q.x.abs());
                }
            }
            a.half_width = w;
            let (mut st, mut sw, mut rl) = (0u8, 0u8, 0u8);
            for p in &sli.paths {
                match p.kind {
                    0 => st += 1,
                    1 => sw += 1,
                    2 => rl += 1,
                    _ => {}
                }
            }
            a.lanes = (st, sw, rl);
            let visible = sli.profiles.iter().any(|p| p.points.len() >= 2) && !sli.only_editor;
            a.category = if !visible {
                Category::Technical
            } else if rl > 0 {
                Category::Tracks
            } else if st > 0 {
                Category::Roads
            } else if sw > 0 {
                Category::Paths
            } else {
                Category::Decoration
            };
        }
    }
    a.haystack = format!("{} {} {} {}", a.name, rel, groups, folder).to_lowercase();
    Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(content: &str, omsi: &str) -> Roots {
        Roots { content: content.into(), omsi: omsi.into() }
    }

    /// A file is owned by the root it lies under, not by the root a lookup was asked of: a
    /// mirrored answer (the installation's copy for what the content folder lacks, or a copy
    /// under a higher-priority content root) still has to be named from its own root, or
    /// `describe` cannot strip a prefix and the object never reaches the catalogue. The two
    /// checks below are the layouts openOMSI runs in, without registering either as a global
    /// content root (which other tests would then see).
    #[test]
    fn a_file_belongs_to_the_root_that_holds_it() {
        // (the content folder inside the installation: the longer prefix is the content one)
        let nested = roots("/game/content", "/game");
        let (root, from_content) = owner_of(Path::new("/game/content/Sceneryobjects/A/x.sco"), &nested).expect("the content folder holds it");
        assert_eq!(root, Path::new("/game/content"), "the content folder's copy is the content folder's");
        assert!(from_content);
        let (root, from_content) = owner_of(Path::new("/game/Sceneryobjects/A/x.sco"), &nested).expect("the installation holds it");
        assert_eq!(root, Path::new("/game"), "the installation's copy is the installation's");
        assert!(!from_content);
        // (the content folder a folder of its own)
        let apart = roots("/home/u/.openomsi/content", "/games/OMSI 2");
        let (root, from_content) = owner_of(Path::new("/home/u/.openomsi/content/Sceneryobjects/A/x.sco"), &apart).expect("the content folder holds it");
        assert_eq!(root, Path::new("/home/u/.openomsi/content"));
        assert!(from_content);
        let (root, from_content) = owner_of(Path::new("/games/OMSI 2/Sceneryobjects/A/x.sco"), &apart).expect("the installation holds it");
        assert_eq!(root, Path::new("/games/OMSI 2"));
        assert!(!from_content);
        // (under neither: nothing the catalogue can name it by)
        assert!(owner_of(Path::new("/elsewhere/x.sco"), &apart).is_none());
    }
}
