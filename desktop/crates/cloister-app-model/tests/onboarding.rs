//! First run on the desktop: the philosophy, sign-in, the service, the Distraction List,
//! Workdays and times, the first Day Plan. Steps already done are skipped.

use cloister_app_model::{
    distraction_list, Onboarding, OnboardingMarker, OnboardingStep as Step, Progress, SignInForm,
    WorkdaysForm, PHILOSOPHY,
};
use cloister_core::Command;

fn nothing_done() -> Progress {
    Progress {
        signed_in: false,
        service_running: false,
        settings_set: false,
    }
}

#[test]
fn a_new_user_reads_the_philosophy_then_walks_every_step() {
    assert!(PHILOSOPHY.len() <= 3, "three screens at most");
    let mut o = Onboarding::new();
    let p = nothing_done();
    let mut seen = vec![o.step()];
    while let Some(step) = o.next(&p) {
        seen.push(step);
    }
    assert_eq!(
        seen,
        [
            Step::Philosophy(0),
            Step::Philosophy(1),
            Step::Philosophy(2),
            Step::SignIn,
            Step::Service,
            Step::DistractionList,
            Step::Workdays,
            Step::DayPlan,
        ]
    );
}

#[test]
fn steps_already_done_are_skipped() {
    let mut o = Onboarding::new();
    let p = Progress {
        signed_in: true,
        service_running: true,
        settings_set: true,
    };
    let mut seen = vec![];
    while let Some(step) = o.next(&p) {
        seen.push(step);
    }
    assert_eq!(
        seen,
        [
            Step::Philosophy(1),
            Step::Philosophy(2),
            Step::DistractionList,
            Step::DayPlan
        ]
    );
}

#[test]
fn back_goes_to_the_previous_step() {
    let mut o = Onboarding::new();
    let p = nothing_done();
    o.next(&p);
    o.next(&p);
    o.next(&p); // SignIn
    assert_eq!(o.back(), Step::Philosophy(2));
    assert_eq!(
        Onboarding::new().back(),
        Step::Philosophy(0),
        "nothing before the first"
    );
}

#[test]
fn workdays_default_to_monday_to_friday_nine_to_five() {
    let form = WorkdaysForm::new("Europe/Amsterdam");
    assert_eq!(
        form.command(),
        Ok(Command::ChangeSettings {
            workdays: vec![1, 2, 3, 4, 5],
            workday_start: "09:00".into(),
            shutdown_time: "17:00".into(),
            time_zone: Some("Europe/Amsterdam".into()),
        })
    );
    assert_eq!(
        form.summary(),
        "Mon to Fri, 09:00 to 17:00. Evening Lockdown starts at 17:00."
    );
}

#[test]
fn workdays_can_be_chosen_and_times_typed() {
    let mut form = WorkdaysForm::new("UTC");
    form.toggle(5);
    form.toggle(6);
    form.workday_start = "8:30".into();
    form.shutdown_time = "16:15".into();
    let Ok(Command::ChangeSettings {
        workdays,
        workday_start,
        shutdown_time,
        ..
    }) = form.command()
    else {
        panic!("{:?}", form.command());
    };
    assert_eq!(workdays, [1, 2, 3, 4, 6]);
    assert_eq!(workday_start, "08:30");
    assert_eq!(shutdown_time, "16:15");
    assert_eq!(
        form.summary(),
        "Mon, Tue, Wed, Thu, Sat, 08:30 to 16:15. Evening Lockdown starts at 16:15."
    );
}

#[test]
fn impossible_settings_get_a_calm_notice() {
    let mut form = WorkdaysForm::new("UTC");
    form.shutdown_time = "25:00".into();
    assert_eq!(
        form.command().unwrap_err().text,
        "Times are written like 09:00 or 17:30."
    );
    form.shutdown_time = "08:00".into();
    assert_eq!(
        form.command().unwrap_err().text,
        "Shutdown Time comes after Workday Start."
    );
    form.shutdown_time = "17:00".into();
    for day in 1..=5 {
        form.toggle(day);
    }
    assert_eq!(
        form.command().unwrap_err().text,
        "Choose at least one Workday."
    );
}

#[test]
fn the_distraction_list_is_on_by_default_with_examples() {
    let list = distraction_list();
    let titles: Vec<_> = list.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["Social", "News", "Video", "Shopping", "Forums"]);
    let video = &list[2];
    assert!(
        video.examples.iter().any(|d| d == "youtube.com"),
        "{:?}",
        video.examples
    );
    assert!(video.examples.len() <= 4);
}

#[test]
fn sign_in_needs_an_email_then_a_six_digit_code() {
    let mut form = SignInForm::default();
    assert!(!form.can_send());
    form.email = " ana@example.com ".into();
    assert!(form.can_send());
    assert_eq!(form.email(), "ana@example.com");
    form.code = "12 34".into();
    assert!(!form.can_verify());
    form.code = "123456".into();
    assert!(form.can_verify());
}

#[test]
fn onboarding_is_shown_once() {
    let dir = tempfile::tempdir().unwrap();
    let marker = OnboardingMarker::new(dir.path().join("Cloister/onboarded"));
    assert!(!marker.is_done());
    marker.mark_done().unwrap();
    assert!(marker.is_done());
}
