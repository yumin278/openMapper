// Frozen from integration 4f33c9f; test oracle only.
use super::*;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyRoadSpec {
    pub name: String,
    /// Lanes the way the road is drawn, and against it (0: one-way).
    pub lanes_fwd: u8,
    pub lanes_back: u8,
    pub lane_width: f32,
    /// A strip between the two ways (m, 0: none).
    pub median: f32,
    /// The strip stands as high as the footways, with a kerb along it (else it lies level with
    /// the roadway).
    pub median_raised: bool,
    /// Footway each side (m, 0: none) and the kerb's height. The footway on the right is the
    /// same unless `footway_right` says otherwise.
    pub footway: f32,
    pub footway_right: Option<f32>,
    pub kerb: f32,
    /// A bus pocket: a lane (m, 0: none) beside the roadway on the right, between it and the
    /// footway, for buses to stop in. A road type made with one is what the Roads tool's bus
    /// pocket tool lays where a bay is wanted.
    pub pocket: f32,
    /// Texture files (found in the installation or the content folder).
    pub roadway_tex: PathBuf,
    pub footway_tex: PathBuf,
    /// The middle strip's own texture (grass, paving); the roadway's runs on under it if none.
    pub median_tex: PathBuf,
    /// The lines are painted by the maker (`markings`) onto the roadway texture, which should
    /// be plain asphalt, instead of being the ones the texture has.
    pub marked: bool,
    pub markings: Vec<Marking>,
    /// What is drawn over them by hand (kept beside the road as `<name>.marks.png`).
    #[serde(skip)]
    pub hand: Option<std::sync::Arc<Hand>>,
}

impl Default for LegacyRoadSpec {
    fn default() -> Self {
        LegacyRoadSpec { name: String::new(), lanes_fwd: 1, lanes_back: 1, lane_width: 3.5, median: 0.0, median_raised: false, footway: 3.0, footway_right: None, kerb: 0.15, pocket: 0.0, roadway_tex: PathBuf::new(), footway_tex: PathBuf::new(), median_tex: PathBuf::new(), marked: false, markings: Vec::new(), hand: None }
    }
}

fn num(v: f32) -> String {
    format!("{v:.3}")
}

/// A ready-made start for the road maker: (name, what it is made of).
pub fn presets() -> Vec<(&'static str, LegacyRoadSpec)> {
    use crate::i18n::tr;
    let base = LegacyRoadSpec::default();
    vec![
        (tr("Quiet street", "Тихая улица"), LegacyRoadSpec { lane_width: 3.0, footway: 2.0, ..base.clone() }),
        (tr("Street with a footway on one side", "Улица с тротуаром с одной стороны"), LegacyRoadSpec { lane_width: 3.0, footway: 0.0, footway_right: Some(2.0), ..base.clone() }),
        (tr("Main road", "Главная дорога"), LegacyRoadSpec { lanes_fwd: 2, lanes_back: 2, lane_width: 3.25, footway: 3.0, ..base.clone() }),
        (tr("Avenue with a green strip", "Проспект с газоном"), LegacyRoadSpec { lanes_fwd: 2, lanes_back: 2, lane_width: 3.25, median: 4.0, median_raised: true, footway: 3.5, ..base.clone() }),
        (tr("One-way street", "Улица с односторонним движением"), LegacyRoadSpec { lanes_fwd: 2, lanes_back: 0, lane_width: 3.0, footway: 2.5, ..base.clone() }),
        (tr("Street with a bus pocket", "Улица с автобусным карманом"), LegacyRoadSpec { lane_width: 3.0, footway: 2.5, pocket: 3.0, ..base.clone() }),
        (tr("Country road", "Просёлочная дорога"), LegacyRoadSpec { lane_width: 3.0, footway: 0.0, kerb: 0.0, ..base.clone() }),
        (tr("Service road", "Служебный проезд"), LegacyRoadSpec { lanes_fwd: 1, lanes_back: 0, lane_width: 3.5, footway: 0.0, kerb: 0.0, ..base }),
    ]
}

impl LegacyRoadSpec {
    pub fn roadway_width(&self) -> f32 {
        (self.lanes_fwd + self.lanes_back) as f32 * self.lane_width + self.median_width()
    }

    /// The middle strip's width (none on a one-way road).
    pub fn median_width(&self) -> f32 {
        if self.lanes_fwd > 0 && self.lanes_back > 0 { self.median } else { 0.0 }
    }

    pub fn footway_left(&self) -> f32 {
        self.footway
    }

    pub fn footway_right(&self) -> f32 {
        self.footway_right.unwrap_or(self.footway)
    }

    pub fn total_width(&self) -> f32 {
        self.roadway_width() + self.pocket + self.footway_left() + self.footway_right()
    }

    /// Where the two ways meet across the roadway (m from its middle): off the middle when one
    /// way has more lanes than the other.
    pub fn centerline(&self) -> f32 {
        (self.lanes_back as f32 - self.lanes_fwd as f32) * self.lane_width / 2.0
    }

    /// The middle strip's left and right edges (m from the roadway's middle), about the centreline.
    pub fn median_edges(&self) -> (f32, f32) {
        let (c, mid) = (self.centerline(), self.median_width());
        (c - mid / 2.0, c + mid / 2.0)
    }

    /// Where a place across the roadway (m from its middle) lies in the roadway texture (0..1).
    pub fn roadway_u(&self, x: f32) -> f32 {
        let rw = self.roadway_width();
        let (xl, mid) = (-rw / 2.0, self.median_width());
        if mid > 0.0 && !self.median_tex.as_os_str().is_empty() {
            let lanes_w = (rw - mid).max(0.1);
            let (ml, mr) = self.median_edges();
            let u_mid = (ml - xl) / lanes_w;
            if x <= ml {
                (x - xl) / lanes_w
            } else if x >= mr {
                u_mid + (x - mr) / lanes_w
            } else {
                u_mid
            }
        } else {
            (x - xl) / rw.max(0.1)
        }
    }

    /// How wide the roadway texture lies across the road (m): the roadway, less a middle strip
    /// of its own, which has its own texture.
    pub fn canvas_width(&self) -> f32 {
        let own = self.median_width() > 0.0 && !self.median_tex.as_os_str().is_empty();
        (self.roadway_width() - if own { self.median_width() } else { 0.0 }).max(0.5)
    }

    /// How far along the road the painted roadway texture runs before it repeats (m): about
    /// ten, a whole number of the first dashed line's periods so its dashes go on unbroken.
    pub fn mark_period(&self) -> f32 {
        let first = self.markings.iter().filter(|m| matches!(m.style, MarkStyle::Dashed | MarkStyle::SolidDashed)).map(|m| (m.dash + m.gap).max(0.5)).next().unwrap_or(10.0);
        (first * (10.0 / first).round().max(1.0)).clamp(2.0, 40.0)
    }

    /// A dashed line's period as painted: the texture's length holds a whole number of them.
    pub fn dash_period(&self, m: &Marking) -> f32 {
        let l = self.mark_period();
        l / (l / (m.dash + m.gap).max(0.5)).round().max(1.0)
    }

    /// The v scale of the roadway's profiles: 0.1 (ten metres to the texture), or the painted
    /// texture's own length.
    fn roadway_v(&self) -> f32 {
        if self.marked { 1.0 / self.mark_period() } else { 0.1 }
    }

    fn roadway_tex_name(&self) -> String {
        if self.marked {
            format!("{}_lines.bmp", self.file_stem())
        } else {
            self.roadway_tex.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()
        }
    }

    /// The markings a road like this usually has: edge lines, the middle (double yellow, or
    /// white edge lines of the strip), dashed lines between lanes of one way.
    pub fn standard_markings(&self) -> Vec<Marking> {
        let (rw, mid, lw) = (self.roadway_width(), self.median_width(), self.lane_width);
        let two_way = self.lanes_fwd > 0 && self.lanes_back > 0;
        let line = |x: f32, style: MarkStyle, color: [u8; 3]| Marking { x, style, color, ..Default::default() };
        let mut v = Vec::new();
        let edge = if self.pocket > 0.0 { MarkStyle::Dashed } else { MarkStyle::Solid };
        let offset = self.centerline();
        v.push(line(-rw / 2.0 + 0.2, MarkStyle::Solid, WHITE));
        v.push(line(rw / 2.0 - 0.2, edge, WHITE));
        if two_way {
            let gap = mid / 2.0;
            if mid <= 0.0 {
                v.push(line(offset, MarkStyle::Double, YELLOW));
            } else {
                v.push(line(offset - gap - 0.2, MarkStyle::Solid, WHITE));
                v.push(line(offset + gap + 0.2, MarkStyle::Solid, WHITE));
            }
            for i in 1..self.lanes_back {
                v.push(line(offset - gap - i as f32 * lw, MarkStyle::Dashed, WHITE));
            }
            for i in 1..self.lanes_fwd {
                v.push(line(offset + gap + i as f32 * lw, MarkStyle::Dashed, WHITE));
            }
        } else {
            for i in 1..(self.lanes_fwd + self.lanes_back) {
                v.push(line(-rw / 2.0 + i as f32 * lw, MarkStyle::Dashed, WHITE));
            }
        }
        v
    }

    /// The roadway texture with the markings painted on: the texture's picture stretched
    /// across the roadway as it is laid (u across, v along `mark_period` metres), lines drawn
    /// with soft edges.
    pub fn painted_roadway(&self) -> Option<image::RgbaImage> {
        let img = omsi_texture::decode_file(&self.roadway_tex).ok()?;
        let src = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.rgba.clone())?;
        let (w, h) = (1024usize, 512usize);
        let mut buf = image::imageops::resize(&src, w as u32, h as u32, image::imageops::FilterType::Triangle);
        let len = self.mark_period();
        for m in &self.markings {
            let period = self.dash_period(m);
            for (x, dashed) in m.lines_at() {
                let (a, b) = (self.roadway_u(x - m.width / 2.0) * w as f32, self.roadway_u(x + m.width / 2.0) * w as f32);
                let (lo, hi) = (a.min(b), a.max(b));
                for px in (lo.floor().max(0.0) as usize)..(hi.ceil().max(0.0) as usize).min(w) {
                    let cover = (hi.min(px as f32 + 1.0) - lo.max(px as f32)).clamp(0.0, 1.0);
                    if cover <= 0.0 {
                        continue;
                    }
                    for py in 0..h {
                        if dashed && ((py as f32 + 0.5) / h as f32 * len) % period >= m.dash.min(period) {
                            continue;
                        }
                        let p = buf.get_pixel_mut(px as u32, py as u32);
                        for c in 0..3 {
                            p.0[c] = (p.0[c] as f32 + (m.color[c] as f32 - p.0[c] as f32) * cover * 0.96) as u8;
                        }
                    }
                }
            }
        }
        if let Some(h) = &self.hand {
            h.over(&mut buf);
        }
        Some(buf)
    }

    /// The file name it is saved as.
    pub fn file_stem(&self) -> String {
        let s: String = self.name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        if s.is_empty() { "road".into() } else { s }
    }

    /// The `.sli` text (OMSI's own keywords; `\n`, turned into CR LF when written).
    pub fn sli(&self, left_hand: bool) -> String {
        let rw = self.roadway_width();
        let (xl, xr) = (-rw / 2.0, rw / 2.0);
        let (fl, fr) = (self.footway_left(), self.footway_right());
        let pw = self.pocket.max(0.0);
        // (the right footway and its kerb start past the pocket)
        let xr2 = xr + pw;
        let road_h = 0.10f32;
        let side_h = road_h + self.kerb;
        let mid = self.median_width();
        let own_median = mid > 0.0 && !self.median_tex.as_os_str().as_encoded_bytes().is_empty();
        let raised = mid > 0.0 && self.median_raised;
        let rv = self.roadway_v();
        let mut s = format!("File created with openMapper\n\n[friendlyname]\n{}\n\n", self.name.trim());
        // the ground and the wheels: footways high, the roadway low
        let height = |s: &mut String, a: f32, b: f32, h: f32| {
            let _ = write!(s, "[heightprofile]\n{}\n{}\n{}\n{}\n\n", num(a), num(b), num(h), num(h));
        };
        if fl > 0.0 {
            height(&mut s, xl - fl, xl, side_h);
        }
        let (ml, mr) = self.median_edges();
        if raised {
            height(&mut s, xl, ml, road_h);
            height(&mut s, ml, mr, side_h);
            height(&mut s, mr, xr, road_h);
        } else {
            height(&mut s, xl, xr, road_h);
        }
        if pw > 0.0 {
            height(&mut s, xr, xr2, road_h);
        }
        if fr > 0.0 {
            height(&mut s, xr2, xr2 + fr, side_h);
        }
        let name = |p: &Path| p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let _ = write!(s, "[texture]\n{}\n\n", self.roadway_tex_name());
        let side_tex = (fl > 0.0 || fr > 0.0 || raised) && !self.footway_tex.as_os_str().is_empty();
        let mut next = 1u32;
        let mut side_ix = 0;
        if side_tex {
            let _ = write!(s, "[texture]\n{}\n\n", name(&self.footway_tex));
            side_ix = next;
            next += 1;
        }
        let mut median_ix = 0;
        if own_median {
            let _ = write!(s, "[texture]\n{}\n\n", name(&self.median_tex));
            median_ix = next;
        }
        let profile = |s: &mut String, tex: u32, a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
            let _ = write!(s, "[profile]\n{tex}\n\n");
            for p in [a, b] {
                let _ = write!(s, "[profilepnt]\n{}\n{}\n{}\n{}\n\n", num(p.0), num(p.1), num(p.2), num(p.3));
            }
        };
        // the roadway: one texture across it, u 0 at its left edge and 1 at its right (as the
        // stock lane textures are painted); a strip of its own texture in the middle takes its
        // own place, the roadway's u running across the lanes either side of it
        if own_median {
            let lanes_w = rw - mid;
            let u_mid = ((ml - xl) / lanes_w).clamp(0.0, 1.0);
            let y = |on: bool| if on { side_h } else { road_h };
            profile(&mut s, 0, (xl, road_h, 0.0, rv), (ml, road_h, u_mid, rv));
            profile(&mut s, 0, (mr, road_h, u_mid, rv), (xr, road_h, 1.0, rv));
            if raised && side_tex {
                profile(&mut s, side_ix, (ml, road_h, 0.995, 0.2), (ml, y(true), 0.953, 0.2));
                profile(&mut s, side_ix, (mr, y(true), 0.953, 0.2), (mr, road_h, 0.995, 0.2));
            }
            profile(&mut s, median_ix, (ml, y(raised), 0.0, mid), (mr, y(raised), 1.0, mid));
        } else {
            profile(&mut s, 0, (xl, road_h, 0.0, rv), (xr, road_h, 1.0, rv));
            if raised && side_tex {
                profile(&mut s, side_ix, (ml, road_h, 0.995, 0.2), (ml, side_h, 0.953, 0.2));
                profile(&mut s, side_ix, (ml, side_h, 0.953, 0.2), (mr, side_h, 0.187, 0.2));
                profile(&mut s, side_ix, (mr, side_h, 0.953, 0.2), (mr, road_h, 0.995, 0.2));
            }
        }
        if side_tex {
            // kerb face and footway top, on the stock footway texture's kerb and pavement parts
            let t = side_ix;
            if fl > 0.0 {
                profile(&mut s, t, (xl - fl, side_h, 0.187, 0.2), (xl, side_h, 0.953, 0.2));
                profile(&mut s, t, (xl, side_h, 0.953, 0.2), (xl, road_h, 0.995, 0.2));
            }
            if fr > 0.0 {
                profile(&mut s, t, (xr2, road_h, 0.995, 0.2), (xr2, side_h, 0.953, 0.2));
                profile(&mut s, t, (xr2, side_h, 0.953, 0.2), (xr2 + fr, side_h, 0.187, 0.2));
            }
        }
        if pw > 0.0 {
            // the pocket: the roadway's own asphalt at the roadway's scale (u 1 is the roadway's
            // width), centred on the middle of its first lane; squeezed into a sliver of the
            // texture it would be a smear of one column across the whole bay
            let lanes = (self.lanes_fwd + self.lanes_back).max(1) as f32;
            let span = pw / rw.max(0.1);
            let u0 = (0.5 / lanes - span / 2.0).clamp(0.0, (1.0 - span).max(0.0));
            profile(&mut s, 0, (xr, road_h, u0, rv), (xr2, road_h, u0 + span, rv));
        }
        // Left-hand traffic reverses the vehicle paths on the same cross-section.
        let forward = u8::from(left_hand);
        let backward = u8::from(!left_hand);
        let mid_gap = mid / 2.0;
        let offset = self.centerline();
        let one_way = self.lanes_back == 0 || self.lanes_fwd == 0;
        if fl > 0.0 {
            let _ = write!(s, "[path]\n1\n{}\n{}\n{}\n2\n\n", num(xl - fl / 2.0), num(side_h), num(fl * 0.4));
        }
        for i in 0..self.lanes_back {
            let x = if one_way { xl + (i as f32 + 0.5) * self.lane_width } else { offset - mid_gap - (self.lanes_back - i) as f32 * self.lane_width + self.lane_width / 2.0 };
            let _ = write!(s, "[path]\n0\n{}\n{}\n{}\n{backward}\n\n", num(x), num(road_h), num(self.lane_width * 0.9));
        }
        for i in 0..self.lanes_fwd {
            let x = if one_way { xl + (i as f32 + 0.5) * self.lane_width } else { offset + mid_gap + (i as f32 + 0.5) * self.lane_width };
            let _ = write!(s, "[path]\n0\n{}\n{}\n{}\n{forward}\n\n", num(x), num(road_h), num(self.lane_width * 0.9));
        }
        if pw > 0.0 {
            // The pocket follows the direction of the forward lane group.
            let _ = write!(s, "[path]\n0\n{}\n{}\n{}\n{forward}\n\n", num(xr + pw / 2.0), num(road_h), num(pw * 0.9));
        }
        if fr > 0.0 {
            let _ = write!(s, "[path]\n1\n{}\n{}\n{}\n2\n\n", num(xr2 + fr / 2.0), num(side_h), num(fr * 0.4));
        }
        s
    }

    /// Write a right-hand road for tests that build spline fixtures.
    #[cfg(test)]
    pub fn write(&self, content: &Path) -> anyhow::Result<String> {
        self.write_with_traffic(content, false)
    }

    /// Write it into the content folder with its textures; returns its relative path
    /// (`Splines\openMapper\<name>.sli`).
    pub fn write_with_traffic(&self, content: &Path, left_hand: bool) -> anyhow::Result<String> {
        anyhow::ensure!(self.lanes_fwd + self.lanes_back > 0, "a road needs a lane");
        anyhow::ensure!(self.roadway_tex.is_file(), "the roadway texture is missing");
        let dir = content.join("Splines").join("openMapper");
        let tex_dir = dir.join("texture");
        std::fs::create_dir_all(&tex_dir)?;
        for t in [&self.roadway_tex, &self.footway_tex, &self.median_tex].into_iter().filter(|t| t.is_file()) {
            let name = t.file_name().unwrap();
            if t.parent() != Some(tex_dir.as_path()) {
                std::fs::copy(t, tex_dir.join(name))?;
                // (its settings and surface sound go with it)
                for ext in ["cfg", "surf"] {
                    let side = PathBuf::from(format!("{}.{ext}", t.display()));
                    if side.is_file() {
                        std::fs::copy(&side, tex_dir.join(format!("{}.{ext}", name.to_string_lossy())))?;
                    }
                }
            }
        }
        let stem = self.file_stem();
        if self.marked {
            let img = self.painted_roadway().ok_or_else(|| anyhow::anyhow!("the roadway texture could not be read"))?;
            image::DynamicImage::ImageRgba8(img).to_rgb8().save(tex_dir.join(self.roadway_tex_name()))?;
        }
        let path = dir.join(format!("{stem}.sli"));
        crate::doc::safe::write_atomic(&path, &omsi_cfg::encode_text(&self.sli(left_hand).replace('\n', "\r\n")))?;
        // (the spec beside it, so the road can be opened in the maker again)
        crate::doc::safe::write_atomic(&dir.join(format!("{stem}.road.json")), serde_json::to_string_pretty(self)?.as_bytes())?;
        let marks = dir.join(format!("{stem}.marks.png"));
        match self.hand.as_ref().filter(|h| self.marked && !h.is_empty()) {
            Some(h) => h.img.save(&marks)?,
            None => {
                let _ = std::fs::remove_file(&marks);
            }
        }
        Ok(format!("Splines\\openMapper\\{stem}.sli"))
    }

    /// The spec a road type was made with here, if it was.
    pub fn of(sli: &Path) -> Option<LegacyRoadSpec> {
        let json = sli.with_extension("road.json");
        let mut spec: LegacyRoadSpec = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
        if let Ok(img) = image::open(sli.with_extension("marks.png")) {
            let img = img.to_rgba8();
            if (img.width(), img.height()) == (HAND_W, HAND_H) {
                spec.hand = Some(std::sync::Arc::new(Hand { img }));
            }
        }
        Some(spec)
    }
}

