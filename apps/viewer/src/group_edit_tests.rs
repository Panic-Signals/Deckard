//! A group's Delete, Cut, Copy and Paste: the group menu's (BUGS.md #32) and SHIFT +
//! ERASE + a group button on the MK3 (#27).

use crate::layout::{Act, apply_acts};

fn app(groups: usize) -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    for _ in 1..groups {
        apply_acts(&mut app, vec![Act::AddGroup]);
    }
    app
}

fn names(app: &crate::App) -> Vec<String> {
    app.doc.as_ref().unwrap().borrow().project().groups.iter().map(|g| g.name.clone()).collect()
}

#[test]
fn delete_takes_the_group_away_in_one_step() {
    let mut app = app(3);
    assert_eq!(app.group, 2);
    apply_acts(&mut app, vec![Act::DeleteGroup(2)]);
    assert_eq!(names(&app), ["", "Group B1"]);
    assert_eq!(app.group, 1, "the focus moves to a group still there");
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Delete Group"));
}

#[test]
fn cut_copy_and_paste_a_group() {
    let mut app = app(3);
    app.doc.as_ref().unwrap().borrow_mut().rename(deckard_doc::Target::Group(1), "Drums").unwrap();
    apply_acts(&mut app, vec![Act::CopyGroup(1)]);
    apply_acts(&mut app, vec![Act::PasteGroup(2)]);
    assert_eq!(names(&app), ["", "Drums", "Drums"]);
    apply_acts(&mut app, vec![Act::CutGroup(1)]);
    assert_eq!(names(&app), ["", "Drums"]);
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Cut Group"));
    apply_acts(&mut app, vec![Act::PasteGroup(0)]);
    assert_eq!(names(&app), ["Drums", "Drums"]);
    assert_eq!(app.doc.as_ref().unwrap().borrow().undo_label(), Some("Paste Group"));
}

/// The MK3: SHIFT + ERASE + a group button.
#[test]
fn shift_erase_and_a_group_button_delete_it() {
    let mut app = app(3);
    app.surface_act(deckard_surface::Action::DeleteGroup(1));
    assert_eq!(names(&app), ["", "Group C1"]);
}

/// Double-clicking a group (its tile in the Ideas view, its row in the group list)
/// mutes it, and again unmutes it (BUGS.md #23); renaming is in its menu.
#[test]
fn double_clicking_a_group_mutes_it() {
    use eframe::egui::{Event, PointerButton, Pos2};
    use egui_kittest::Harness;
    fn double_click(h: &mut Harness<'static, crate::App>, pos: Pos2) {
        // (Apart from the clicks before: not a triple-click.)
        h.run_steps(40);
        h.hover_at(pos);
        h.step();
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() });
                h.step();
            }
        }
        h.run_steps(2);
    }
    for (song_view, what) in [(false, "group tile 1"), (true, "group row 1")] {
        let Some(mut h) = crate::tests::harness(false, song_view) else { return };
        let muted = |h: &Harness<'static, crate::App>| h.state().doc.as_ref().unwrap().borrow().project().groups[1].muted;
        let at =
            h.ctx.data(|d| d.get_temp::<eframe::egui::Rect>(crate::layout::header_rect(what))).unwrap_or_else(|| panic!("{what} drawn"));
        assert!(!muted(&h));
        double_click(&mut h, at.left_center() + eframe::egui::vec2(12.0, 0.0));
        assert!(muted(&h), "{what}: muted");
        assert_eq!(h.state().renaming, None, "{what}: not renaming");
        double_click(&mut h, at.left_center() + eframe::egui::vec2(12.0, 0.0));
        assert!(!muted(&h), "{what}: unmuted");
    }
}
