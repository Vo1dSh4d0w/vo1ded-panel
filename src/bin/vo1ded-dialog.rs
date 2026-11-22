//! Shows a popup dialog and prints the user's answer on stdout.

use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

use clap::{Parser, ValueEnum};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use vo1ded_panel::dialog::{self, Dialog, DialogAction, Response, Urgency};

/// Exit status for a failure to show the dialog (clap uses the same for
/// invalid arguments).
const EXIT_ERROR: u8 = 2;

/// Shows a popup dialog and prints the answer on stdout: `action:KEY` if an
/// action was pressed (exit status 0), `dismissed` if the dialog was
/// dismissed with Escape or its dismiss button (exit status 1).
#[derive(Debug, Parser)]
#[command(name = "vo1ded-dialog", version)]
struct Args {
    /// Title of the dialog.
    title: String,

    /// Body text.
    #[arg(short, long, default_value = "")]
    body: String,

    /// Interpret the body as markup (<b>, <i>, <u>, <a href>).
    #[arg(short, long)]
    markup: bool,

    /// Icon name, absolute path or file:// URI.
    #[arg(short, long)]
    icon: Option<String>,

    /// Action button as KEY=LABEL, or KEY to use it as the label too.
    /// Repeatable; buttons appear in order.
    #[arg(short, long = "action", value_name = "KEY=LABEL", value_parser = parse_action)]
    actions: Vec<DialogAction>,

    /// Urgency, which sets the dialog's colors.
    #[arg(short, long, value_enum, default_value_t = UrgencyArg::Normal)]
    urgency: UrgencyArg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum UrgencyArg {
    Low,
    Normal,
    Critical,
}

impl From<UrgencyArg> for Urgency {
    fn from(urgency: UrgencyArg) -> Self {
        match urgency {
            UrgencyArg::Low => Urgency::Low,
            UrgencyArg::Normal => Urgency::Normal,
            UrgencyArg::Critical => Urgency::Critical,
        }
    }
}

fn parse_action(spec: &str) -> Result<DialogAction, String> {
    let (key, label) = spec.split_once('=').unwrap_or((spec, spec));
    if key.is_empty() {
        return Err("the action key must not be empty".into());
    }
    Ok(DialogAction::new(key, label))
}

impl From<Args> for Dialog {
    fn from(args: Args) -> Self {
        Dialog {
            title: args.title,
            body: args.body,
            markup: args.markup,
            icon: args.icon,
            actions: args.actions,
            urgency: args.urgency.into(),
        }
    }
}

fn main() -> ExitCode {
    let dialog: Dialog = Args::parse().into();

    vo1ded_panel::init();
    // Several dialogs may be open at once, each its own process.
    let app = gtk::Application::new(Some("dev.vo1ded.dialog"), gio::ApplicationFlags::NON_UNIQUE);
    let response: Rc<Cell<Option<Response>>> = Rc::default();
    let r = response.clone();
    app.connect_activate(move |app| {
        let Some(display) = gdk::Display::default() else {
            eprintln!("vo1ded-dialog: no display");
            return;
        };
        vo1ded_panel::load_assets(&display);
        let (app, dialog, r) = (app.clone(), dialog.clone(), r.clone());
        // Keep running until the dialog is answered.
        let hold = app.hold();
        glib::spawn_future_local(async move {
            r.set(Some(dialog::run(&app, &dialog).await));
            drop(hold);
        });
    });
    // The arguments were ours, not GTK's.
    app.run_with_args::<&str>(&[]);

    match response.take() {
        Some(Response::Action(key)) => {
            println!("action:{key}");
            ExitCode::SUCCESS
        }
        Some(Response::Dismissed) => {
            println!("dismissed");
            ExitCode::from(1)
        }
        None => ExitCode::from(EXIT_ERROR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Dialog, clap::Error> {
        Args::try_parse_from(std::iter::once("vo1ded-dialog").chain(args.iter().copied()))
            .map(Into::into)
    }

    #[test]
    fn parses_all_options() {
        let dialog = parse(&[
            "-b",
            "body",
            "--markup",
            "--icon=dialog-question",
            "-a",
            "yes=Yes",
            "-a",
            "no",
            "-u",
            "critical",
            "Title",
        ])
        .unwrap();
        assert_eq!(
            dialog,
            Dialog {
                title: "Title".into(),
                body: "body".into(),
                markup: true,
                icon: Some("dialog-question".into()),
                actions: vec![
                    DialogAction::new("yes", "Yes"),
                    DialogAction::new("no", "no")
                ],
                urgency: Urgency::Critical,
            }
        );
    }

    #[test]
    fn label_may_contain_equals() {
        let dialog = parse(&["-a", "k=a=b", "T"]).unwrap();
        assert_eq!(dialog.actions, vec![DialogAction::new("k", "a=b")]);
    }

    #[test]
    fn rejects_invalid_arguments() {
        for args in [
            &[][..],
            &["-b"],
            &["-a", "=x", "T"],
            &["-u", "high", "T"],
            &["A", "B"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }
}
