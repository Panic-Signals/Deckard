//! Clips in the app (MK3 manual, "Working with Clips"): ⌘. and Section ▸ Convert to
//! Clip(s) make them; the Song view selects, moves, resizes, duplicates, doubles and
//! deletes them. The document's side is `deckard_doc`'s clips.

use crate::App;
use crate::layout::ClipAct;

impl App {
    /// ⌘.: the focused group's pattern in the picked section (the Song view) becomes a
    /// clip over the section, and is selected.
    pub fn convert_focused_to_clip(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        if !self.song_view {
            self.status = "Convert to Clip works in the Song view".into();
            return;
        }
        let g = self.group;
        let found = {
            let d = doc.borrow();
            let p = d.project();
            self.section.and_then(|i| {
                let sec = p.sections.get(i)?;
                let pi = p.scenes.get(sec.scene?)?.patterns.iter().find(|(pg, _)| *pg == g)?.1;
                Some((pi, p.section_start(i), sec.length))
            })
        };
        let Some((pi, start, length)) = found else {
            self.status = "pick a section whose scene plays a pattern of the focused group".into();
            return;
        };
        let r = doc.borrow_mut().convert_to_clip(g, pi, start, length);
        match r {
            Ok(c) => self.clip = Some((g, c)),
            Err(e) => self.status = format!("can't convert to a clip: {e}"),
        }
    }

    /// Section ▸ Convert to Clip(s).
    pub fn convert_section_to_clips(&mut self, i: usize) {
        let Some(doc) = self.doc.clone() else { return };
        let r = doc.borrow_mut().convert_section_to_clips(i);
        if let Err(e) = r {
            self.status = format!("can't convert to clips: {e}");
        }
    }

    /// What the Song view's clips ask for.
    pub fn clip_act(&mut self, g: usize, c: usize, a: ClipAct) {
        let Some(doc) = self.doc.clone() else { return };
        let Some(clip) = doc.borrow().project().groups.get(g).and_then(|x| x.clips.get(c)).copied() else { return };
        let r = match a {
            ClipAct::Select => {
                self.clip = Some((g, c));
                if g != self.group {
                    self.focus_group(g);
                }
                Ok(())
            }
            ClipAct::Move(to) => doc.borrow_mut().set_clip(g, c, to, clip.length),
            ClipAct::Length(len) => doc.borrow_mut().set_clip(g, c, clip.position, len),
            ClipAct::Duplicate => {
                let r = doc.borrow_mut().duplicate_clip(g, c);
                r.map(|k| self.clip = Some((g, k)))
            }
            ClipAct::Double => doc.borrow_mut().double_clip(g, c),
            ClipAct::Delete => {
                self.clip = None;
                doc.borrow_mut().delete_clip(g, c)
            }
        };
        // A moved clip keeps its selection (its index may change with the order).
        if matches!(a, ClipAct::Move(_)) {
            let d = doc.borrow();
            self.clip = d.project().groups[g].clips.iter().position(|x| x.pattern == clip.pattern).map(|k| (g, k));
        }
        if let Err(e) = r {
            self.status = format!("can't edit the clip: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::App;
    use crate::layout::{Act, ClipAct, apply_acts};
    use deckard_model::{Clip, Note, PPQ};
    use eframe::egui;

    /// A new project in the Song view: pattern 1 with a note, in scene 1, in two
    /// sections; the second picked.
    fn app() -> App {
        let mut app = App::default();
        app.new_project();
        let doc = app.doc.clone().unwrap();
        {
            let mut d = doc.borrow_mut();
            if d.project().groups[0].patterns.is_empty() {
                d.add_pattern(0, usize::MAX, "Pattern 1").unwrap();
            }
            let n = Note { expr: None, sound: 0, position: 0, length: PPQ, pitch: 60, velocity: 100, probability: 100 };
            d.edit_notes("Notes", 0, 0, &[], &[n]).unwrap();
            d.assign_pattern(0, 0, Some(0)).unwrap();
            d.add_section(0).unwrap();
            d.add_section(0).unwrap();
        }
        app.song_view = true;
        app.section = Some(1);
        app
    }

    fn clips(app: &App) -> Vec<Clip> {
        app.doc.as_ref().unwrap().borrow().project().groups[0].clips.clone()
    }

    /// ⌘. converts the focused group's pattern in the picked section, and selects it;
    /// the clip's menu and drags edit it.
    #[test]
    fn command_period_converts_and_the_song_view_edits() {
        let mut app = app();
        let bar = 4 * PPQ;
        apply_acts(&mut app, vec![Act::ConvertToClip]);
        assert_eq!(clips(&app), [Clip { position: bar, length: bar, pattern: 0 }], "{}", app.status);
        assert_eq!(app.clip, Some((0, 0)));
        apply_acts(&mut app, vec![Act::Clip(0, 0, ClipAct::Move(3 * bar))]);
        apply_acts(&mut app, vec![Act::Clip(0, 0, ClipAct::Length(2 * bar))]);
        assert_eq!(clips(&app), [Clip { position: 3 * bar, length: 2 * bar, pattern: 0 }]);
        apply_acts(&mut app, vec![Act::Clip(0, 0, ClipAct::Duplicate)]);
        assert_eq!(clips(&app).len(), 2);
        assert_eq!(app.clip, Some((0, 1)));
        apply_acts(&mut app, vec![Act::Clip(0, 1, ClipAct::Delete)]);
        assert_eq!(clips(&app).len(), 1);
        apply_acts(&mut app, vec![Act::ConvertSectionToClips(0)]);
        assert_eq!(clips(&app)[0], Clip { position: 0, length: bar, pattern: 1 });
        // The Ideas view doesn't.
        app.song_view = false;
        apply_acts(&mut app, vec![Act::ConvertToClip]);
        assert_eq!(clips(&app).len(), 2);
    }

    /// The key: ⌘. in the window.
    #[test]
    fn the_key() {
        let mut h =
            egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app: &mut App| app.frame(ui), app());
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Period);
        h.run_steps(2);
        assert_eq!(clips(h.state()).len(), 1, "{}", h.state().status);
        // DECKARD_PICTURE=file.png: a picture of the Song view with the clip.
        if let Ok(out) = std::env::var("DECKARD_PICTURE") {
            h.run_steps(2);
            h.render().unwrap().save(out).unwrap();
        }
    }
}
