//! Depot files (`.hof`): the destinations a bus of the map shows, the stop names its
//! displays and announcements use, and the routes its IBIS knows (line and route code, the
//! stops in order). OMSI 2 takes them from the map's folder; openOMSI from the bus's folder
//! and the content's `HOFs` folder, so saving puts a copy there too.
//!
//! Read as openOMSI reads them (`omsi-vehicle`'s `hof.rs`), written in the stock files'
//! layout. Comments of a file are not kept when it is written again.

use std::path::Path;

use omsi_cfg::{CfgFile, KeywordRule};

use crate::i18n::tr;
use crate::timetable::Timetable;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Terminus {
    /// Destination (Ziel) code; extended displays may use more than three digits.
    pub code: i32,
    /// The destination's name: the stop where everybody leaves, and what a timetable
    /// trip's terminus is matched against.
    pub ident: String,
    /// Nobody gets on (a service trip, an empty sign).
    pub all_exit: bool,
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stop {
    /// The timetable's name of the stop.
    pub ident: String,
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Route {
    /// Line number and two digits ("7601": line 76, its first route).
    pub code: String,
    /// What the IBIS shows for it ("BAUERNHOF-KRANKENHAUS").
    pub name: String,
    /// The destination's code.
    pub terminus: String,
    /// The line as the timetable names it.
    pub line: String,
    pub stops: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Hof {
    /// The file's name without `.hof`.
    pub file: String,
    pub name: String,
    pub service_trip: String,
    pub global_strings: Vec<String>,
    pub terminus_strings: usize,
    pub stop_strings: usize,
    pub termini: Vec<Terminus>,
    pub stops: Vec<Stop>,
    pub routes: Vec<Route>,
}

/// What a destination's texts are for, as OMSI 2's own buses read them (Grundorf's and
/// Spandau's depot files: eight texts).
pub fn terminus_label(i: usize) -> String {
    match i {
        0 => tr("IBIS and roller blind", "IBIS и роллбанд").into(),
        1 => tr("Front sign, line 1", "Передний указатель, строка 1").into(),
        2 => tr("Front sign, line 2", "Передний указатель, строка 2").into(),
        3 => tr("Side sign", "Боковой указатель").into(),
        4 => tr("Roller blind texture", "Текстура роллбанда").into(),
        5 => tr("IBIS 2 (20 letters)", "IBIS 2 (20 знаков)").into(),
        6 => tr("Extra sign", "Доп. табличка").into(),
        7 => tr("Krüger picture", "Картинка Krüger").into(),
        _ => format!("{} {}", tr("Text", "Текст"), i + 1),
    }
}

/// What a stop's texts are for (OMSI 2's buses: four).
pub fn stop_label(i: usize) -> String {
    match i {
        0 => tr("IBIS (16 letters)", "IBIS (16 знаков)").into(),
        1 => tr("Display, line 1", "Табло, строка 1").into(),
        2 => tr("Display, line 2", "Табло, строка 2").into(),
        3 => tr("Full name (20 letters)", "Полное имя (20 знаков)").into(),
        _ => format!("{} {}", tr("Text", "Текст"), i + 1),
    }
}

impl Hof {
    pub fn load(path: &Path) -> Option<Hof> {
        let f = CfgFile::read(path).ok()?;
        let mut h = parse(&f);
        h.file = path.file_stem()?.to_string_lossy().into_owned();
        Some(h)
    }

    /// The file's name in the folder.
    pub fn file_name(&self) -> String {
        format!("{}.hof", self.file)
    }

    /// A new depot file with the stock layout of texts and the three signs every OMSI 2
    /// depot has (empty, service trip, special trip).
    pub fn new(file: &str) -> Hof {
        let service = tr("Service trip", "В парк").to_string();
        let special = tr("Special trip", "Заказной").to_string();
        let mut h = Hof { file: file.into(), name: file.into(), service_trip: service.clone(), global_strings: vec![String::new(); 4], terminus_strings: 8, stop_strings: 4, ..Default::default() };
        let blank = Terminus { code: 0, ident: tr("Blank", "Пусто").into(), all_exit: true, strings: vec![String::new(); 8] };
        h.termini.push(blank);
        for (code, n) in [(1, service), (2, special)] {
            let mut t = terminus_for(&h, code, &n);
            t.all_exit = true;
            h.termini.push(t);
        }
        h
    }

    /// The destination of this name (the timetable's terminus), as openOMSI finds it.
    pub fn terminus_named(&self, name: &str) -> Option<&Terminus> {
        let n = name.trim();
        self.termini.iter().find(|t| t.ident.trim().eq_ignore_ascii_case(n))
    }

    pub fn free_code(&self) -> i32 {
        let mut c = self.termini.iter().map(|t| t.code).max().unwrap_or(0) + 1;
        if c < 3 {
            c = 3;
        }
        c
    }

    /// Add what the map's lines need and the file lacks: a destination for each trip's
    /// terminus, every stop the trips call at, and a route for each way a line goes
    /// (the stops in order). What is there is kept as it is. How many of each were added.
    pub fn fill_from(&mut self, tt: &Timetable) -> (usize, usize, usize) {
        let mut added = (0, 0, 0);
        let mut free_line = 900;
        for trip in &tt.trips {
            let mut stops: Vec<String> = trip.stations.iter().filter_map(|id| tt.stop_name(*id)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            // (two stations of one name in a row, Grundorf's Bauernhof: one stop to the IBIS)
            stops.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            if stops.len() < 2 {
                continue;
            }
            let terminus = if trip.terminus.trim().is_empty() { stops.last().unwrap().clone() } else { trip.terminus.trim().to_string() };
            if self.terminus_named(&terminus).is_none() {
                let t = terminus_for(self, self.free_code(), &terminus);
                self.termini.push(t);
                added.0 += 1;
            }
            for s in &stops {
                if !self.stops.iter().any(|x| x.ident.trim().eq_ignore_ascii_case(s)) {
                    let st = stop_for(self, s);
                    self.stops.push(st);
                    added.1 += 1;
                }
            }
            let line = trip.line.trim().to_string();
            let same = |r: &Route| r.line.trim().eq_ignore_ascii_case(&line) && r.stops.len() == stops.len() && r.stops.iter().zip(&stops).all(|(a, b)| a.trim().eq_ignore_ascii_case(b));
            if self.routes.iter().any(same) {
                continue;
            }
            // the code a driver types: the line's number and the route's two digits; a line
            // without a number gets one of its own from 900 up
            let number = line_number(&line).or_else(|| self.routes.iter().find(|r| r.line.trim().eq_ignore_ascii_case(&line)).and_then(|r| r.code.trim().parse::<u32>().ok()).map(|c| c / 100)).unwrap_or_else(|| {
                while self.routes.iter().any(|r| r.code.trim().parse::<u32>().ok().map(|c| c / 100) == Some(free_line)) {
                    free_line += 1;
                }
                free_line
            });
            let mut k = 1;
            while self.routes.iter().any(|r| r.code.trim().parse::<u32>().ok() == Some(number * 100 + k)) && k < 99 {
                k += 1;
            }
            let code = self.terminus_named(&terminus).map(|t| t.code).unwrap_or(0);
            let name = format!("{}-{}", short(&stops[0].to_uppercase(), 16), short(&terminus.to_uppercase(), 16));
            self.routes.push(Route { code: (number * 100 + k).to_string(), name, terminus: code.to_string(), line, stops });
            added.2 += 1;
        }
        added
    }
}

/// The number a line starts with ("76", "M5" has none).
fn line_number(line: &str) -> Option<u32> {
    let d: String = line.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok().filter(|n| *n > 0 && *n < 10000)
}

/// At most `n` letters.
fn short(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A name in two lines of at most `n` letters each, split at the space nearest the middle
/// ("Einsteindorf Marktplatz": "Einsteindorf", "Marktplatz"); one line when it fits.
pub fn two_lines(s: &str, n: usize) -> (String, String) {
    let s = s.trim();
    if s.chars().count() <= n {
        return (s.to_string(), String::new());
    }
    let mid = s.chars().count() / 2;
    let split = s.char_indices().filter(|(_, c)| *c == ' ').map(|(i, _)| i).min_by_key(|i| s[..*i].chars().count().abs_diff(mid));
    match split {
        Some(i) => (short(s[..i].trim(), n), short(s[i..].trim(), n)),
        None => (short(s, n), String::new()),
    }
}

/// Centred in `n` letters (the stock files centre the front signs' text).
fn centred(s: &str, n: usize) -> String {
    let s = short(s.trim(), n);
    let pad = (n - s.chars().count()) / 2;
    format!("{}{}", " ".repeat(pad), s)
}

/// A destination's texts made from its name, in the depot's layout.
pub fn terminus_for(h: &Hof, code: i32, name: &str) -> Terminus {
    let up = name.trim().to_uppercase();
    let (a, b) = two_lines(&up, 16);
    let mut strings = vec![String::new(); h.terminus_strings];
    let mut set = |i: usize, v: String| {
        if let Some(s) = strings.get_mut(i) {
            *s = v;
        }
    };
    if h.terminus_strings == 8 {
        set(0, short(&up, 16));
        if b.is_empty() {
            set(2, centred(&a, 16));
        } else {
            set(1, centred(&a, 16));
            set(2, centred(&b, 16));
        }
        set(3, centred(&short(&up, 16), 16));
        set(5, short(name.trim(), 20));
    } else {
        for i in 0..h.terminus_strings {
            set(i, short(&up, 16));
        }
    }
    Terminus { code, ident: name.trim().to_string(), all_exit: false, strings }
}

/// A stop's texts made from its name, in the depot's layout.
pub fn stop_for(h: &Hof, name: &str) -> Stop {
    let (a, b) = two_lines(name, 16);
    let mut strings = vec![String::new(); h.stop_strings];
    let vals = [short(&name.trim().to_uppercase(), 16), a, b, short(name.trim(), 20)];
    for (s, v) in strings.iter_mut().zip(vals) {
        *s = v;
    }
    Stop { ident: name.trim().to_string(), strings }
}

pub fn parse(f: &CfgFile) -> Hof {
    let mut h = Hof::default();
    // the counts are bare lines, not keywords in brackets
    for (i, l) in f.lines.iter().enumerate() {
        let w = l.trim_end();
        let n = || f.lines.get(i + 1).map(|s| omsi_cfg::parse_i64(s).max(0) as usize).unwrap_or(0);
        if w.eq_ignore_ascii_case("stringcount_terminus") {
            h.terminus_strings = n();
        } else if w.eq_ignore_ascii_case("stringcount_busstop") {
            h.stop_strings = n();
        }
    }
    let mut r = f.reader().with_rule(KeywordRule::TrimEnd);
    while let Some(k) = r.next_keyword() {
        match k.as_str() {
            "name" => h.name = r.str().to_string(),
            "servicetrip" => h.service_trip = r.str().to_string(),
            "global_strings" => {
                let n = r.usize();
                h.global_strings = (0..n).map(|_| r.str().to_string()).collect();
            }
            "addterminus" | "addterminus_allexit" => {
                let all_exit = k.ends_with("allexit");
                let code = r.i32();
                let ident = r.str().to_string();
                let strings = (0..h.terminus_strings).map(|_| r.str().to_string()).collect();
                h.termini.push(Terminus { code, ident, all_exit, strings });
            }
            "addterminus_list" => {
                for l in r.until("[end]") {
                    if l.trim().is_empty() {
                        continue;
                    }
                    let cols: Vec<&str> = l.split('\t').collect();
                    let all_exit = cols[0].trim().eq_ignore_ascii_case("{ALLEX}");
                    let code = omsi_cfg::parse_i32(cols.get(1).unwrap_or(&"0"));
                    let ident = cols.get(2).unwrap_or(&"").trim().to_string();
                    let mut strings: Vec<String> = cols.iter().skip(3).map(|s| s.trim_end().to_string()).collect();
                    strings.resize(h.terminus_strings, String::new());
                    h.termini.push(Terminus { code, ident, all_exit, strings });
                }
            }
            "addbusstop" => {
                let ident = r.str().to_string();
                let strings = (0..h.stop_strings).map(|_| r.str().to_string()).collect();
                h.stops.push(Stop { ident, strings });
            }
            "addbusstop_list" => {
                for l in r.until("[end]") {
                    if l.trim().is_empty() {
                        continue;
                    }
                    let mut cols = l.split('\t');
                    let ident = cols.next().unwrap_or("").trim().to_string();
                    let mut strings: Vec<String> = cols.map(|s| s.trim_end().to_string()).collect();
                    strings.resize(h.stop_strings, String::new());
                    h.stops.push(Stop { ident, strings });
                }
            }
            "infosystem_trip" => {
                let code = r.str().trim().to_string();
                let name = r.str().to_string();
                let terminus = r.str().trim().to_string();
                let line = r.str().to_string();
                h.routes.push(Route { code, name, terminus, line, stops: Vec::new() });
            }
            "infosystem_busstop_list" => {
                let n = r.usize();
                let list = (0..n).map(|_| r.str().to_string()).collect();
                if let Some(last) = h.routes.last_mut() {
                    last.stops = list;
                }
            }
            _ => {}
        }
    }
    h
}

/// What every file openMapper writes says (another one of the name in `HOFs` is left alone).
pub const MARK: &str = "Created with openMapper";

/// The file's text, in the stock files' layout.
pub fn text(h: &Hof) -> String {
    let mut o = format!("\t########################\n\t\tHOF\n\t########################\n\n\t{MARK} {}\n\n", env!("CARGO_PKG_VERSION"));
    o.push_str(&format!("[name]\n{}\n\n", h.name.trim()));
    if !h.service_trip.trim().is_empty() {
        o.push_str(&format!("[servicetrip]\n{}\n\n", h.service_trip.trim()));
    }
    if !h.global_strings.is_empty() {
        o.push_str(&format!("[global_strings]\n{}\n", h.global_strings.len()));
        for s in &h.global_strings {
            o.push_str(&format!("{}\n", s.trim_end()));
        }
        o.push('\n');
    }
    o.push_str(&format!("stringcount_terminus\n{}\n\nstringcount_busstop\n{}\n\n", h.terminus_strings, h.stop_strings));
    for t in &h.termini {
        o.push_str(if t.all_exit { "[addterminus_allexit]\n" } else { "[addterminus]\n" });
        o.push_str(&format!("{}\n{}\n", t.code, t.ident.trim()));
        for i in 0..h.terminus_strings {
            o.push_str(&format!("{}\n", t.strings.get(i).map(|s| s.trim_end()).unwrap_or("")));
        }
        o.push_str("................\n\n");
    }
    for s in &h.stops {
        o.push_str(&format!("[addbusstop]\n{}\n", s.ident.trim()));
        for i in 0..h.stop_strings {
            o.push_str(&format!("{}\n", s.strings.get(i).map(|s| s.trim_end()).unwrap_or("")));
        }
        o.push_str("................\n\n");
    }
    for r in &h.routes {
        o.push_str(&format!("[infosystem_trip]\n{}\n{}\n{}\n{}\n\n", r.code.trim(), r.name.trim_end(), r.terminus.trim(), r.line.trim_end()));
        o.push_str(&format!("[infosystem_busstop_list]\n{}\n", r.stops.len()));
        for s in &r.stops {
            o.push_str(&format!("{}\n", s.trim()));
        }
        o.push('\n');
    }
    o
}

/// The depot files of the map folder `dir`.
pub fn load_dir(dir: &Path) -> Vec<Hof> {
    let mut v: Vec<Hof> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("hof")).unwrap_or(false))
        .filter_map(|p| Hof::load(&p))
        .collect();
    v.sort_by_key(|h| h.file.to_lowercase());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stock_depot_reads_back_the_same() {
        let Some(omsi) = crate::install::find_omsi_root(&crate::install::Prefs::default()) else { return };
        let mut n = 0;
        for d in std::fs::read_dir(omsi.join("Vehicles")).into_iter().flatten().flatten() {
            for h in load_dir(&d.path()) {
                let again = parse(&CfgFile::from_str("x.hof", &text(&h)));
                assert_eq!(again, Hof { file: String::new(), ..h.clone() }, "{}", h.file);
                n += 1;
            }
        }
        let g = Hof::load(&omsi.join("Vehicles/MAN_SD202/Grundorf.hof")).unwrap();
        assert!(g.termini.len() > 5 && g.routes.len() > 1 && g.routes[0].stops.len() == 7, "{g:?}");
        assert!(n > 10, "{n}");
    }


    #[test]
    fn extended_ziel_codes_survive_read_and_write() {
        let input = "stringcount_terminus\n2\n\nstringcount_busstop\n0\n\n[addterminus_list]\n\t106\tRomerberg\tBASE\t\n\t20106\tRomerberg\tGLETSCHER-XPRESS*I*K\tRomerberg ZOB*B\n\t21106\tRomerberg\tGLETSCHER-XPRESS*I*K\tvia Zavelstein\n\t14990\t\t*B*CE\t*CE\n[end]\n";
        let hof = parse(&CfgFile::from_str("extended.hof", input));
        let output = text(&hof);
        println!("Before:\n{input}\nAfter:\n{output}");
        assert_eq!(hof.termini.iter().map(|t| t.code).collect::<Vec<_>>(), vec![106, 20106, 21106, 14990]);
        assert_eq!(parse(&CfgFile::from_str("saved.hof", &output)), hof);
    }

    #[test]
    fn names_in_two_lines() {
        assert_eq!(two_lines("Einsteindorf Marktplatz", 16), ("Einsteindorf".into(), "Marktplatz".into()));
        assert_eq!(two_lines("Hbf", 16), ("Hbf".into(), String::new()));
        assert_eq!(centred("AB", 6), "  AB");
    }

    #[test]
    fn routes_from_the_lines() {
        let mut tt = Timetable::default();
        for (id, n) in [(1, "Bauernhof"), (2, "Elsterplatz"), (3, "Krankenhaus")] {
            tt.stops.push(omsi_timetable::BusStopEntry { name: n.into(), group: 0, object_id: id, params: [0.0; 3] });
        }
        let trip = |name: &str, stations: Vec<i64>, terminus: &str| omsi_timetable::Trip { name: name.into(), line: "76".into(), terminus: terminus.into(), stations, ..Default::default() };
        tt.trips.push(trip("a", vec![1, 1, 2, 3], "Krankenhaus"));
        tt.trips.push(trip("b", vec![3, 2, 1], "Bauernhof"));
        tt.trips.push(trip("c", vec![1, 2, 3], "Krankenhaus"));
        let mut h = Hof::new("Test");
        assert_eq!(h.fill_from(&tt), (2, 3, 2));
        assert_eq!(h.routes[0].code, "7601");
        assert_eq!(h.routes[1].code, "7602");
        assert_eq!(h.routes[0].stops, ["Bauernhof", "Elsterplatz", "Krankenhaus"]);
        assert_eq!(h.routes[0].terminus, h.terminus_named("Krankenhaus").unwrap().code.to_string());
        assert_eq!(h.fill_from(&tt), (0, 0, 0));
        let again = parse(&CfgFile::from_str("t.hof", &text(&h)));
        assert_eq!(again, Hof { file: String::new(), ..h });
    }
}
