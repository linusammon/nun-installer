use super::*;
use installer_core::{Reachability, SafetyError};

fn catalog() -> Catalog {
    Catalog::example()
}

fn host() -> Host {
    Host::example()
}

fn disk(name: &str, rejected: Vec<SafetyError>) -> DiskInfo {
    DiskInfo {
        rejected,
        ..DiskInfo::example(name)
    }
}

fn two_disks() -> Vec<DiskInfo> {
    vec![
        disk(
            "sda",
            vec![SafetyError::Mounted {
                device: "/dev/sda1".into(),
                target: "/boot".into(),
            }],
        ),
        disk("vda", vec![]),
    ]
}

fn online() -> ChecksResult {
    ChecksResult {
        internet: Reachability {
            github: true,
            cache: true,
        },
        clock_note: None,
        on_ac: None,
        missing_tools: vec![],
    }
}

fn app() -> App {
    App::new(catalog(), host(), false, two_disks)
}

/// An app whose Welcome checks have passed.
fn ready() -> App {
    let mut a = app();
    a.start();
    a.on_checks(online());
    a
}

fn press(app: &mut App, code: KeyCode) -> Vec<Effect> {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c));
    }
}

fn message(app: &App) -> String {
    app.errors()
        .iter()
        .map(|e| e.message.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

/// Walk to the Timezone screen: welcome, then the keyboard (us, no variants).
fn to_timezone(app: &mut App) {
    press(app, KeyCode::Enter); // welcome
    press(app, KeyCode::Enter); // keyboard
    assert_eq!(app.screen(), Screen::Timezone);
}

/// Walk to the Disk screen, keeping defaults.
fn to_disk(app: &mut App) {
    to_timezone(app);
    press(app, KeyCode::Enter); // timezone
    press(app, KeyCode::Enter); // locale
}

/// Walk to the User screen: first usable disk, no swap, no encryption.
fn to_user(app: &mut App) {
    to_disk(app);
    assert_eq!(app.screen(), Screen::Disk);
    press(app, KeyCode::Enter); // disk -> swap
    press(app, KeyCode::Enter); // swap -> encrypt
    press(app, KeyCode::Enter); // encryption is off, so this submits
    assert_eq!(app.screen(), Screen::User);
}

fn to_summary(app: &mut App) {
    to_user(app);
    type_str(app, "alice");
    press(app, KeyCode::Tab);
    type_str(app, "hunter2");
    press(app, KeyCode::Tab);
    type_str(app, "hunter2");
    press(app, KeyCode::Enter);
    assert_eq!(app.screen(), Screen::Summary);
}

#[test]
fn start_requests_the_checks() {
    let mut a = app();
    let fx = a.start();
    assert!(matches!(fx[..], [Effect::RunChecks { internet: true }]));
    assert!(a.checks_running());
}

#[test]
fn welcome_blocks_until_checks_pass() {
    let mut a = app();
    a.start();
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Welcome, "still running");

    a.on_checks(ChecksResult {
        internet: Reachability {
            github: true,
            cache: false,
        },
        ..online()
    });
    assert!(!a.can_continue());
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Welcome, "no internet");

    a.on_checks(online());
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Keyboard);
}

#[test]
fn welcome_needs_a_usable_disk() {
    let mut a = App::new(catalog(), host(), false, || vec![two_disks().remove(0)]);
    a.start();
    a.on_checks(online());
    assert!(!a.has_usable_disk());
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Welcome);
}

#[test]
fn offline_mode_skips_the_internet_requirement() {
    let mut a = App::new(catalog(), host(), true, two_disks);
    let fx = a.start();
    assert!(matches!(fx[..], [Effect::RunChecks { internet: false }]));
    a.on_checks(ChecksResult {
        internet: Reachability {
            github: false,
            cache: false,
        },
        ..online()
    });
    assert!(a.can_continue());
}

#[test]
fn r_rechecks_and_q_quits() {
    let mut a = ready();
    let fx = press(&mut a, KeyCode::Char('r'));
    assert!(matches!(fx[..], [Effect::RunChecks { internet: true }]));
    assert!(a.checks_running());
    assert!(!a.can_continue());
    let fx = press(&mut a, KeyCode::Char('q'));
    assert!(matches!(fx[..], [Effect::Quit]));
}

#[test]
fn typing_filters_and_choice_reaches_answers() {
    let mut a = ready();
    to_timezone(&mut a);
    type_str(&mut a, "zur");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Locale);
    assert_eq!(a.answers().system.timezone, "Europe/Zurich");
}

#[test]
fn esc_clears_filter_before_going_back() {
    let mut a = ready();
    to_timezone(&mut a);
    type_str(&mut a, "zur");
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.screen(), Screen::Timezone);
    assert_eq!(a.timezone().filter(), "");
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.screen(), Screen::Keyboard);
}

#[test]
fn empty_filter_result_cannot_be_submitted() {
    let mut a = ready();
    to_timezone(&mut a);
    type_str(&mut a, "zzz");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Timezone);
    assert!(!a.errors().is_empty());
}

fn to_keyboard(a: &mut App) {
    press(a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Keyboard);
}

#[test]
fn layout_without_variants_confirms_straight_away_and_applies_the_keymap() {
    let mut a = ready();
    to_keyboard(&mut a);
    let fx = press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Timezone);
    let [Effect::ApplyKeymap { layout, variant }] = &fx[..] else {
        panic!("expected keymap effect, got {fx:?}");
    };
    assert_eq!((layout.as_str(), variant.as_deref()), ("us", None));
}

#[test]
fn variant_list_follows_the_highlighted_layout() {
    let mut a = ready();
    to_keyboard(&mut a);
    assert_eq!(
        a.keyboard().variant_list().visible().len(),
        1,
        "us has only (default)"
    );
    type_str(&mut a, "switz");
    assert_eq!(a.keyboard().layout_value(), "ch");
    let names: Vec<_> = a
        .keyboard()
        .variant_list()
        .visible()
        .iter()
        .map(|i| i.value.clone())
        .collect();
    assert_eq!(names, ["", "fr"]);
    assert_eq!(a.keyboard().layout_description(), "German (Switzerland)");
}

#[test]
fn enter_on_a_layout_with_variants_moves_to_the_variant_pane_then_confirms() {
    let mut a = ready();
    to_keyboard(&mut a);
    type_str(&mut a, "switz");
    assert!(press(&mut a, KeyCode::Enter).is_empty());
    assert_eq!(a.screen(), Screen::Keyboard);
    assert_eq!(a.keyboard().pane(), Pane::Variant);

    type_str(&mut a, "french");
    let fx = press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Timezone);
    let [Effect::ApplyKeymap { layout, variant }] = &fx[..] else {
        panic!("expected keymap effect, got {fx:?}");
    };
    assert_eq!((layout.as_str(), variant.as_deref()), ("ch", Some("fr")));
    assert_eq!(a.answers().system.keyboard_variant.as_deref(), Some("fr"));
}

#[test]
fn default_variant_means_none() {
    let mut a = ready();
    to_keyboard(&mut a);
    type_str(&mut a, "switz");
    press(&mut a, KeyCode::Enter);
    let fx = press(&mut a, KeyCode::Enter);
    assert!(matches!(
        &fx[..],
        [Effect::ApplyKeymap { variant: None, .. }]
    ));
    assert_eq!(a.answers().system.keyboard_variant, None);
}

#[test]
fn tab_switches_panes_and_esc_steps_back_through_them() {
    let mut a = ready();
    to_keyboard(&mut a);
    type_str(&mut a, "switz");
    press(&mut a, KeyCode::Tab);
    assert_eq!(a.keyboard().pane(), Pane::Variant);
    type_str(&mut a, "fr");
    press(&mut a, KeyCode::Esc);
    assert_eq!(
        a.keyboard().variant_list().filter(),
        "",
        "esc clears the filter first"
    );
    assert_eq!(a.keyboard().pane(), Pane::Variant);
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.keyboard().pane(), Pane::Layout);
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.keyboard().layout_list().filter(), "");
    press(&mut a, KeyCode::Esc);
    assert_eq!(a.screen(), Screen::Welcome);
}

#[test]
fn typing_goes_to_the_focused_pane_only() {
    let mut a = ready();
    to_keyboard(&mut a);
    press(&mut a, KeyCode::Tab);
    type_str(&mut a, "x");
    assert_eq!(a.keyboard().variant_list().filter(), "x");
    assert_eq!(a.keyboard().layout_list().filter(), "");
}

#[test]
fn disk_picker_starts_on_first_usable_disk_and_refuses_rejected_ones() {
    let mut a = ready();
    to_disk(&mut a);
    assert_eq!(a.screen(), Screen::Disk);
    assert_eq!(a.disk().focus(), DiskField::Disk);
    assert_eq!(a.disk().selected().unwrap().name, "vda");
    press(&mut a, KeyCode::Up);
    assert_eq!(a.disk().selected().unwrap().name, "sda");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.disk().focus(), DiskField::Disk, "stays on the list");
    assert!(message(&a).contains("mounted"));
    assert_eq!(a.errors()[0].target, Target::Disk);
}

#[test]
fn swap_defaults_to_none() {
    let a = app();
    assert_eq!(a.disk().swap().value(), "0");
    assert_eq!(a.answers().disk.swap_size_mib, 0);
}

#[test]
fn enter_moves_through_the_fields_and_only_the_last_one_submits() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        (a.screen(), a.disk().focus()),
        (Screen::Disk, DiskField::Swap)
    );
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        (a.screen(), a.disk().focus()),
        (Screen::Disk, DiskField::Encrypt)
    );
    press(&mut a, KeyCode::Enter);
    assert_eq!(
        a.screen(),
        Screen::User,
        "encryption is off, so this was the last field"
    );
}

#[test]
fn enter_on_swap_rejects_a_non_number_without_moving_on() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Backspace);
    assert_eq!(a.disk().swap().value(), "");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.disk().focus(), DiskField::Swap);
    assert!(a.error_for(Target::Swap).is_some());
}

#[test]
fn arrows_pick_disks_in_the_list_and_move_between_fields_elsewhere() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Down);
    assert_eq!(
        a.disk().selected().unwrap().name,
        "vda",
        "stops at the last disk"
    );
    assert_eq!(a.disk().focus(), DiskField::Disk);
    press(&mut a, KeyCode::Tab);
    assert_eq!(a.disk().focus(), DiskField::Swap);
    press(&mut a, KeyCode::Up);
    assert_eq!(a.disk().focus(), DiskField::Disk);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Down);
    assert_eq!(a.disk().focus(), DiskField::Encrypt);
    press(&mut a, KeyCode::BackTab);
    assert_eq!(a.disk().focus(), DiskField::Swap);
}

#[test]
fn r_rescans_only_while_the_list_is_focused() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Char('r'));
    assert_eq!(a.disk().swap().value(), "0", "no stray text in the list");
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Char('r'));
    assert_eq!(
        a.disk().swap().value(),
        "0",
        "r is not a digit, so the swap field ignores it"
    );
}

#[test]
fn encryption_fields_appear_only_when_enabled() {
    let mut a = ready();
    to_disk(&mut a);
    assert_eq!(a.disk().fields().len(), 3);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Tab);
    assert_eq!(a.disk().focus(), DiskField::Encrypt);
    press(&mut a, KeyCode::Char(' '));
    assert!(a.disk().encrypt());
    assert_eq!(a.disk().fields().len(), 5);
    press(&mut a, KeyCode::Char(' '));
    assert!(!a.disk().encrypt());
    assert_eq!(a.disk().fields().len(), 3);
}

#[test]
fn passphrase_rules_are_enforced_on_the_right_field() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Char(' '));
    press(&mut a, KeyCode::Enter); // encrypt -> passphrase
    press(&mut a, KeyCode::Enter); // passphrase -> confirm
    press(&mut a, KeyCode::Enter); // confirm submits
    assert_eq!(a.screen(), Screen::Disk, "empty passphrase");
    assert!(a.error_for(Target::Passphrase).is_some());
    assert_eq!(
        a.disk().focus(),
        DiskField::Passphrase,
        "focus jumps to the problem"
    );

    type_str(&mut a, "longenough1");
    press(&mut a, KeyCode::Tab);
    type_str(&mut a, "different11");
    press(&mut a, KeyCode::Enter);
    assert!(a
        .error_for(Target::PassphraseConfirm)
        .is_some_and(|m| m.contains("do not match")));
    assert_eq!(a.disk().focus(), DiskField::Confirm);
    for _ in 0..11 {
        press(&mut a, KeyCode::Backspace);
    }
    type_str(&mut a, "longenough1");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::User);
    assert_eq!(
        a.answers().disk.encryption_password.unwrap().expose(),
        "longenough1"
    );
}

#[test]
fn typing_a_swap_size_replaces_the_default_and_takes_digits_only() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Tab);
    type_str(&mut a, "2a0b48");
    assert_eq!(a.disk().swap().value(), "2048");
    assert_eq!(a.answers().disk.swap_size_mib, 2048);
}

#[test]
fn submitting_with_a_bad_disk_focuses_the_list() {
    let mut a = ready();
    to_disk(&mut a);
    press(&mut a, KeyCode::Up); // sda, which is in use
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Tab);
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Disk);
    assert_eq!(a.disk().focus(), DiskField::Disk);
    assert!(message(&a).contains("mounted"));
}

#[test]
fn user_screen_checks_name_and_matching_passwords_then_keeps_only_the_hash() {
    let mut a = ready();
    to_user(&mut a);

    type_str(&mut a, "Alice");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::User);
    assert!(a.error_for(Target::Username).is_some());

    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    type_str(&mut a, "alice");
    press(&mut a, KeyCode::Tab);
    type_str(&mut a, "one");
    press(&mut a, KeyCode::Tab);
    type_str(&mut a, "two");
    press(&mut a, KeyCode::Enter);
    assert!(a
        .error_for(Target::PasswordConfirm)
        .is_some_and(|m| m.contains("doesn't match")));

    for _ in 0..3 {
        press(&mut a, KeyCode::Backspace);
    }
    type_str(&mut a, "one");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Summary);
    assert!(a.user().password().is_empty() && a.user().password_confirm().is_empty());
    assert!(a.answers().user.password_hash.expose().starts_with("$y$"));
}

#[test]
fn enter_on_the_username_moves_to_the_password_without_complaining() {
    let mut a = ready();
    to_user(&mut a);
    type_str(&mut a, "alice");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::User);
    assert_eq!(a.user().focus(), UserField::Password);
    assert!(
        a.errors().is_empty(),
        "no password error yet: {:?}",
        a.errors()
    );

    type_str(&mut a, "hunter2");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.user().focus(), UserField::Confirm);
    assert!(a.errors().is_empty());

    type_str(&mut a, "hunter2");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::Summary, "the last field submits");
}

#[test]
fn enter_on_a_bad_username_or_empty_password_stays_on_that_field() {
    let mut a = ready();
    to_user(&mut a);
    type_str(&mut a, "Alice");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.user().focus(), UserField::Name);
    assert!(a.error_for(Target::Username).is_some());

    for _ in 0..5 {
        press(&mut a, KeyCode::Backspace);
    }
    type_str(&mut a, "alice");
    press(&mut a, KeyCode::Enter);
    press(&mut a, KeyCode::Enter); // empty password
    assert_eq!(a.user().focus(), UserField::Password);
    assert!(a.error_for(Target::Password).is_some());
}

#[test]
fn a_failed_submit_puts_the_cursor_on_the_confirm_field() {
    let mut a = ready();
    to_user(&mut a);
    type_str(&mut a, "alice");
    press(&mut a, KeyCode::Enter);
    type_str(&mut a, "one");
    press(&mut a, KeyCode::Enter);
    type_str(&mut a, "two");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.screen(), Screen::User);
    assert_eq!(a.user().focus(), UserField::Confirm);
    assert!(a.error_for(Target::PasswordConfirm).is_some());
}

#[test]
fn the_hostname_is_fixed() {
    assert_eq!(app().answers().system.hostname, "nun");
}

#[test]
fn install_needs_the_disk_name_typed_and_yields_a_plan() {
    let mut a = ready();
    to_summary(&mut a);
    assert_eq!(a.confirm_word(), "vda");
    assert!(press(&mut a, KeyCode::Enter).is_empty());
    assert_eq!(a.screen(), Screen::Summary);
    assert!(a.error_for(Target::Confirm).is_some());

    type_str(&mut a, "vda");
    let fx = press(&mut a, KeyCode::Enter);
    let [Effect::Install(plan)] = &fx[..] else {
        panic!("expected install, got {fx:?}");
    };
    assert_eq!(a.screen(), Screen::Installing);
    assert!(plan
        .actions
        .iter()
        .any(|x| x.description.contains("Partition, format and mount")));
    let real = plan.actions.iter().filter(|x| !x.cleanup).count();
    assert_eq!(
        a.install().steps().len(),
        real,
        "cleanup is not shown as a step"
    );
    assert!(real < plan.actions.len());
}

#[test]
fn a_disk_appearing_before_install_cannot_change_what_gets_wiped() {
    use std::cell::Cell;
    use std::rc::Rc;
    let plugged = Rc::new(Cell::new(false));
    let flag = plugged.clone();
    let mut a = App::new(catalog(), host(), false, move || {
        let mut disks = Vec::new();
        if flag.get() {
            // A USB stick: sorts before vda, so index 0 now means a different disk.
            disks.push(disk("sda", vec![]));
        }
        disks.push(disk("vda", vec![]));
        disks
    });
    a.start();
    a.on_checks(online());
    to_summary(&mut a);
    assert_eq!(a.confirm_word(), "vda");
    type_str(&mut a, "vda");

    plugged.set(true);
    let fx = press(&mut a, KeyCode::Enter);

    let [Effect::Install(plan)] = &fx[..] else {
        panic!("expected an install of the confirmed disk, got {fx:?}");
    };
    let wiped: Vec<_> = plan
        .actions
        .iter()
        .filter(|x| x.description.starts_with("Partition, format and mount"))
        .map(|x| x.description.as_str())
        .collect();
    assert_eq!(wiped, ["Partition, format and mount /dev/vda"]);
    assert!(!format!("{plan:?}").contains("/dev/sda"));
}

#[test]
fn install_refuses_if_the_confirmed_disk_is_gone_even_if_another_one_remains() {
    use std::cell::Cell;
    use std::rc::Rc;
    let unplugged = Rc::new(Cell::new(false));
    let flag = unplugged.clone();
    let mut a = App::new(catalog(), host(), false, move || {
        if flag.get() {
            vec![disk("sda", vec![])]
        } else {
            vec![disk("vda", vec![])]
        }
    });
    a.start();
    a.on_checks(online());
    to_summary(&mut a);
    type_str(&mut a, "vda");

    unplugged.set(true);
    assert!(press(&mut a, KeyCode::Enter).is_empty());
    assert_eq!(a.screen(), Screen::Disk, "sent back to choose again");
    assert!(message(&a).contains("/dev/vda is no longer available"));
    assert_eq!(
        a.summary().confirm().value(),
        "",
        "the old word is forgotten"
    );
}

#[test]
fn install_refuses_if_disk_became_unusable() {
    use std::cell::Cell;
    use std::rc::Rc;
    let ok = Rc::new(Cell::new(true));
    let flag = ok.clone();
    let mut a = App::new(catalog(), host(), false, move || {
        let rejected = if flag.get() {
            vec![]
        } else {
            vec![SafetyError::SwapActive("/dev/vda".into())]
        };
        vec![disk("vda", rejected)]
    });
    a.start();
    a.on_checks(online());
    to_summary(&mut a);
    type_str(&mut a, "vda");
    ok.set(false);
    assert!(press(&mut a, KeyCode::Enter).is_empty());
    assert_eq!(a.screen(), Screen::Disk);
    assert!(message(&a).contains("no longer available"));
}

#[test]
fn ctrl_c_quits_except_while_installing() {
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    let mut a = app();
    assert!(matches!(a.handle_key(ctrl_c)[..], [Effect::Quit]));
    a.jump_to(Screen::Installing);
    assert!(a.handle_key(ctrl_c).is_empty());
}

#[test]
fn install_events_drive_progress_and_finish() {
    let mut a = ready();
    to_summary(&mut a);
    type_str(&mut a, "vda");
    press(&mut a, KeyCode::Enter);

    a.on_install_event(InstallEvent::Step(2));
    assert_eq!(a.install().steps()[0].1, StepStatus::Done);
    assert_eq!(a.install().steps()[2].1, StepStatus::Running);
    assert_eq!(a.install().steps()[3].1, StepStatus::Pending);
    a.on_install_event(InstallEvent::Output("hello".into()));
    assert_eq!(a.install().log(), ["hello"]);
    a.on_install_event(InstallEvent::Finished(Err("boom".into())));
    assert_eq!(a.screen(), Screen::Done);
    assert_eq!(a.install().steps()[2].1, StepStatus::Failed);
    assert_eq!(
        a.install().steps()[3].1,
        StepStatus::Pending,
        "steps after the failure never ran"
    );
    assert!(matches!(press(&mut a, KeyCode::Enter)[..], [Effect::Quit]));
}

#[test]
fn success_offers_reboot() {
    let mut a = app();
    a.jump_to(Screen::Installing);
    a.set_install_steps(vec![("x".into(), StepStatus::Running)]);
    a.on_install_event(InstallEvent::Finished(Ok(())));
    assert_eq!(a.install().steps()[0].1, StepStatus::Done);
    assert!(matches!(
        press(&mut a, KeyCode::Enter)[..],
        [Effect::Reboot]
    ));
}

#[test]
fn keymap_result_becomes_a_notice() {
    let mut a = app();
    a.on_keymap_result(Ok("ch (fr)".into()));
    assert_eq!(a.notice(), Some("Console keyboard set to ch (fr)"));
    a.on_keymap_result(Err("not a console".into()));
    assert!(a.notice().unwrap().contains("not a console"));
}

#[test]
fn every_screen_has_exactly_one_step_in_the_bar() {
    // Adding a Screen without listing it here is a compile error, and
    // without adding it to STEPS a test failure.
    let all = [
        Screen::Welcome,
        Screen::Keyboard,
        Screen::Timezone,
        Screen::Locale,
        Screen::Disk,
        Screen::User,
        Screen::Summary,
        Screen::Installing,
        Screen::Done,
    ];
    for screen in all {
        match screen {
            Screen::Welcome
            | Screen::Keyboard
            | Screen::Timezone
            | Screen::Locale
            | Screen::Disk
            | Screen::User
            | Screen::Summary
            | Screen::Installing
            | Screen::Done => {}
        }
        assert_eq!(
            STEPS.iter().filter(|(s, _)| *s == screen).count(),
            1,
            "{screen:?}"
        );
    }
    assert_eq!(STEPS.len(), all.len());
    let order: Vec<_> = STEPS.iter().map(|(s, _)| *s).collect();
    assert_eq!(order, all, "walked through in the listed order");
    assert_eq!(STEPS[Screen::Welcome.step()].1, "Welcome");
    assert_eq!(STEPS[Screen::Summary.step()].1, "Review");
}
