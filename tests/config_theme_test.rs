use nuvem::config::theme::{Theme, ThemeChoice};

#[test]
fn no_color_theme_uses_no_colors() {
    let t = Theme::no_color();
    for s in [
        t.app_title,
        t.border,
        t.selected_row,
        t.status_error,
        t.status_healthy,
        t.key_hint,
    ] {
        assert!(s.fg.is_none(), "no-color theme must not set fg");
        assert!(s.bg.is_none(), "no-color theme must not set bg");
    }
}

#[test]
fn high_contrast_differs_from_default() {
    assert_ne!(Theme::high_contrast().status_error.fg, None);
    assert_ne!(
        Theme::high_contrast().selected_row,
        Theme::default_theme().selected_row
    );
}

#[test]
fn for_choice_maps_variants() {
    assert_eq!(
        Theme::for_choice(ThemeChoice::Default).app_title,
        Theme::default_theme().app_title
    );
    assert_eq!(
        Theme::for_choice(ThemeChoice::NoColor).border,
        Theme::no_color().border
    );
}

#[test]
fn choice_parses_leniently() {
    assert_eq!(
        ThemeChoice::from_str_lenient("no-color"),
        Some(ThemeChoice::NoColor)
    );
    assert_eq!(
        ThemeChoice::from_str_lenient("HIGH_CONTRAST"),
        Some(ThemeChoice::HighContrast)
    );
    assert_eq!(ThemeChoice::from_str_lenient("bogus"), None);
}

#[test]
fn active_theme_can_be_swapped() {
    nuvem::config::theme::set_active(Theme::high_contrast());
    assert_eq!(
        nuvem::config::theme::active().status_error.fg,
        Theme::high_contrast().status_error.fg
    );
    nuvem::config::theme::set_active(Theme::default_theme());
}
