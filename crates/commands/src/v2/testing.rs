//! The test rule of the contract, as code: "every command has a description, an input
//! and an output schema, and its undo is tested". A domain's tests give one example of
//! each of its commands to [`check`]; the root tests prove that no command lacks an
//! example. The harness is generic so that its own rejection branches are tested with
//! deliberately broken commands (see the tests at the bottom).

use crate::v2::contract::{CommandError, Context, Kind, Meta, Outcome, State, Undo, UndoKind};
use std::fmt::Debug;

/// What the harness needs from a command: the root [`crate::v2::Command`], or a fake.
pub trait Checked: Clone + Debug + PartialEq + Sized {
    type Output: Debug;
    fn meta(&self) -> Meta;
    fn apply(
        &self,
        state: &mut State,
        ctx: &mut Context,
    ) -> Result<Outcome<Self, Self::Output>, CommandError>;
    /// The command name an output answers.
    fn output_name(output: &Self::Output) -> &'static str;
}

impl Checked for crate::v2::Command {
    type Output = crate::v2::Output;
    fn meta(&self) -> Meta {
        Self::meta(self)
    }
    fn apply(
        &self,
        state: &mut State,
        ctx: &mut Context,
    ) -> Result<Outcome<Self, Self::Output>, CommandError> {
        Self::apply(self, state, ctx)
    }
    fn output_name(output: &Self::Output) -> &'static str {
        output.name()
    }
}

/// Checks one example of a command against the contract, from `state`:
///
/// - its metadata are consistent ([`Meta::check`]);
/// - it applies, and its output answers to its name;
/// - `live` leaves the project as it was; `query` leaves everything as it was;
///   `edit` changes the state and reports at least one event;
/// - `undo: inverse`: the inverse is an `edit`, applying it restores the state
///   exactly and reports events, and the inverse's own inverse (redo) brings the
///   edited state back and reports events;
///   `undo: none`: the outcome carries no inverse.
pub fn check<C: Checked>(command: &C, state: &State, ctx: &Context) -> Result<(), String> {
    let meta = command.meta();
    let name = meta.name;
    meta.check()?;
    let before = state.clone();
    let mut state = state.clone();
    let mut ctx = ctx.clone();
    let outcome = command
        .apply(&mut state, &mut ctx)
        .map_err(|e| format!("{name}: the example does not apply: {e}"))?;
    let answers = C::output_name(&outcome.output);
    if answers != name {
        return Err(format!("{name}: the output answers to {answers}"));
    }
    match meta.kind {
        Kind::Live if state.project != before.project => {
            return Err(format!("{name}: a live command changed the project"));
        }
        Kind::Query if state != before => {
            return Err(format!("{name}: a query changed the state"));
        }
        Kind::Edit if outcome.events.is_empty() => {
            return Err(format!("{name}: an edit reports at least one event"));
        }
        Kind::Edit if state == before => {
            return Err(format!("{name}: an edit changed nothing"));
        }
        _ => {}
    }
    match (meta.undo, outcome.undo) {
        (UndoKind::None, Undo::None) => Ok(()),
        (UndoKind::None, Undo::Inverse(_)) => Err(format!(
            "{name}: declared not undoable but returned an inverse"
        )),
        (UndoKind::Inverse, Undo::None) => {
            Err(format!("{name}: declared undoable but returned no inverse"))
        }
        (UndoKind::Inverse, Undo::Inverse(inverse)) => {
            let inverse_meta = inverse.meta();
            if inverse_meta.kind != Kind::Edit {
                return Err(format!(
                    "{name}: its inverse {} is not an edit",
                    inverse_meta.name
                ));
            }
            let edited = state.clone();
            let undone = inverse
                .apply(&mut state, &mut ctx)
                .map_err(|e| format!("{name}: its inverse {} fails: {e}", inverse_meta.name))?;
            if C::output_name(&undone.output) != inverse_meta.name {
                return Err(format!(
                    "{name}: the output of its inverse answers to {}",
                    C::output_name(&undone.output)
                ));
            }
            if state != before {
                return Err(format!(
                    "{name}: its inverse {} does not restore the state:\n before {before:#?}\n after undo {state:#?}",
                    inverse_meta.name
                ));
            }
            if undone.events.is_empty() {
                return Err(format!(
                    "{name}: its inverse {} reports no event",
                    inverse_meta.name
                ));
            }
            let Undo::Inverse(redo) = undone.undo else {
                return Err(format!(
                    "{name}: its inverse {} has no inverse (no redo)",
                    inverse_meta.name
                ));
            };
            let redone = redo
                .apply(&mut state, &mut ctx)
                .map_err(|e| format!("{name}: redo {} fails: {e}", redo.meta().name))?;
            if C::output_name(&redone.output) != redo.meta().name {
                return Err(format!(
                    "{name}: the output of its redo answers to {}",
                    C::output_name(&redone.output)
                ));
            }
            if state != edited {
                return Err(format!(
                    "{name}: redo {} does not bring the edit back",
                    redo.meta().name
                ));
            }
            if redone.events.is_empty() {
                return Err(format!(
                    "{name}: redo {} reports no event",
                    redo.meta().name
                ));
            }
            Ok(())
        }
    }
}

/// Checks that a command is refused and that the refusal leaves the state untouched:
/// a command never half-applies.
pub fn check_refusal<C: Checked>(
    command: &C,
    state: &State,
    ctx: &Context,
) -> Result<CommandError, String> {
    let name = command.meta().name;
    let mut after = state.clone();
    let mut ctx = ctx.clone();
    match command.apply(&mut after, &mut ctx) {
        Ok(_) => Err(format!("{name}: expected a refusal, the command applied")),
        Err(error) if after != *state => Err(format!(
            "{name}: refused with {error} but the state changed:\n before {state:#?}\n after {after:#?}"
        )),
        Err(error) => Ok(error),
    }
}

/// The context the examples run with.
pub fn context() -> Context {
    Context::new(crate::v2::Author::User, project::IdGenerator::for_tests())
}

/// The state the examples run from: a valid project (a project has a name).
pub fn state() -> State {
    State {
        project: project::Project {
            name: "Untitled".into(),
        },
        session: project::SessionState::default(),
    }
}

#[cfg(test)]
mod tests {
    //! Each rejection branch of the harness, hit by a deliberately broken command.
    use super::*;
    use crate::v2::contract::ProjectEvent;

    /// A fake command whose behaviour is chosen by the test.
    #[derive(Clone, Debug, PartialEq)]
    enum Fake {
        /// Renames the project to `to`; `inverse` is what it claims undoes it;
        /// `kind`/`undo` are what it declares.
        Rename {
            to: String,
            kind: Kind,
            undo: UndoKind,
            inverse: Option<Box<Fake>>,
            events: bool,
            answers: &'static str,
            destructive: bool,
        },
        /// Refuses; `taint` changes the project first.
        Refuse { taint: bool },
        /// Does nothing, declared as `kind`.
        Noop { kind: Kind },
    }

    #[derive(Debug)]
    struct Out(&'static str);

    impl Checked for Fake {
        type Output = Out;
        fn meta(&self) -> Meta {
            let (kind, undo, destructive) = match self {
                Fake::Rename {
                    kind,
                    undo,
                    destructive,
                    ..
                } => (*kind, *undo, *destructive),
                Fake::Refuse { .. } => (Kind::Edit, UndoKind::Inverse, false),
                Fake::Noop { kind } => (*kind, UndoKind::None, false),
            };
            Meta {
                name: "fake.cmd",
                kind,
                undo,
                destructive,
            }
        }
        fn apply(
            &self,
            state: &mut State,
            _ctx: &mut Context,
        ) -> Result<Outcome<Self, Out>, CommandError> {
            match self {
                Fake::Rename {
                    to,
                    inverse,
                    events,
                    answers,
                    ..
                } => {
                    state.project.name = to.clone();
                    Ok(Outcome {
                        output: Out(answers),
                        undo: match inverse {
                            Some(i) => Undo::Inverse(i.clone()),
                            None => Undo::None,
                        },
                        events: if *events {
                            vec![ProjectEvent::ProjectRenamed { name: to.clone() }]
                        } else {
                            vec![]
                        },
                        control: vec![],
                    })
                }
                Fake::Refuse { taint } => {
                    if *taint {
                        state.project.name = "tainted".into();
                    }
                    Err(CommandError::Unsupported {
                        command: "fake.cmd".into(),
                    })
                }
                Fake::Noop { .. } => Ok(Outcome {
                    output: Out("fake.cmd"),
                    undo: Undo::None,
                    events: vec![],
                    control: vec![],
                }),
            }
        }
        fn output_name(output: &Out) -> &'static str {
            output.0
        }
    }

    fn rename(to: &str, inverse: Option<Fake>) -> Fake {
        Fake::Rename {
            to: to.into(),
            kind: Kind::Edit,
            undo: UndoKind::Inverse,
            inverse: inverse.map(Box::new),
            events: true,
            answers: "fake.cmd",
            destructive: false,
        }
    }

    fn good() -> Fake {
        rename("new", Some(rename("Untitled", Some(rename("new", None)))))
    }

    fn run(fake: &Fake) -> Result<(), String> {
        check(fake, &state(), &context())
    }

    #[test]
    fn a_correct_edit_passes() {
        run(&good()).unwrap();
    }

    #[test]
    fn inconsistent_metadata_are_refused() {
        let fake = Fake::Rename {
            to: "x".into(),
            kind: Kind::Live,
            undo: UndoKind::Inverse,
            inverse: None,
            events: true,
            answers: "fake.cmd",
            destructive: false,
        };
        assert!(run(&fake).unwrap_err().contains("cannot carry an inverse"));
    }

    #[test]
    fn an_example_that_does_not_apply_is_refused() {
        assert!(
            run(&Fake::Refuse { taint: false })
                .unwrap_err()
                .contains("does not apply")
        );
    }

    #[test]
    fn an_output_answering_another_name_is_refused() {
        let mut fake = good();
        if let Fake::Rename { answers, .. } = &mut fake {
            *answers = "other.cmd";
        }
        assert!(run(&fake).unwrap_err().contains("answers to other.cmd"));
    }

    #[test]
    fn a_live_command_touching_the_project_is_refused() {
        let fake = Fake::Rename {
            to: "x".into(),
            kind: Kind::Live,
            undo: UndoKind::None,
            inverse: None,
            events: false,
            answers: "fake.cmd",
            destructive: false,
        };
        assert!(
            run(&fake)
                .unwrap_err()
                .contains("live command changed the project")
        );
    }

    #[test]
    fn a_query_touching_the_state_is_refused() {
        let fake = Fake::Rename {
            to: "x".into(),
            kind: Kind::Query,
            undo: UndoKind::None,
            inverse: None,
            events: false,
            answers: "fake.cmd",
            destructive: false,
        };
        assert!(run(&fake).unwrap_err().contains("query changed the state"));
        run(&Fake::Noop { kind: Kind::Query }).unwrap();
        run(&Fake::Noop { kind: Kind::Live }).unwrap();
    }

    #[test]
    fn an_edit_without_events_is_refused() {
        let mut fake = good();
        if let Fake::Rename { events, .. } = &mut fake {
            *events = false;
        }
        assert!(run(&fake).unwrap_err().contains("at least one event"));
    }

    #[test]
    fn undo_declaration_and_outcome_must_agree() {
        let mut fake = good();
        if let Fake::Rename { inverse, .. } = &mut fake {
            *inverse = None;
        }
        assert!(run(&fake).unwrap_err().contains("returned no inverse"));
        let fake = Fake::Rename {
            to: "x".into(),
            kind: Kind::Edit,
            undo: UndoKind::None,
            inverse: Some(Box::new(rename("Untitled", None))),
            events: true,
            answers: "fake.cmd",
            destructive: false,
        };
        // Edit + none is inconsistent unless destructive, so the metadata check fires first.
        assert!(
            run(&fake)
                .unwrap_err()
                .contains("must be declared destructive")
        );
        let fake = Fake::Rename {
            to: "x".into(),
            kind: Kind::Edit,
            undo: UndoKind::None,
            inverse: Some(Box::new(rename("Untitled", None))),
            events: true,
            answers: "fake.cmd",
            destructive: true,
        };
        assert!(run(&fake).unwrap_err().contains("returned an inverse"));
        let mut irreversible = fake;
        if let Fake::Rename { inverse, .. } = &mut irreversible {
            *inverse = None;
        }
        run(&irreversible).unwrap();
    }

    #[test]
    fn a_redo_that_fails_is_refused() {
        let fake = rename(
            "new",
            Some(rename("Untitled", Some(Fake::Refuse { taint: false }))),
        );
        assert!(run(&fake).unwrap_err().contains("redo fake.cmd fails"));
    }

    #[test]
    fn an_edit_that_changes_nothing_is_refused() {
        let fake = rename(
            "Untitled",
            Some(rename("Untitled", Some(rename("Untitled", None)))),
        );
        assert!(run(&fake).unwrap_err().contains("changed nothing"));
    }

    #[test]
    fn an_inverse_or_redo_answering_another_name_is_refused() {
        let mut inverse = rename("Untitled", Some(rename("new", None)));
        if let Fake::Rename { answers, .. } = &mut inverse {
            *answers = "other.cmd";
        }
        let err = run(&rename("new", Some(inverse))).unwrap_err();
        assert!(
            err.contains("output of its inverse answers to other.cmd"),
            "{err}"
        );
        let mut redo = rename("new", None);
        if let Fake::Rename { answers, .. } = &mut redo {
            *answers = "other.cmd";
        }
        let err = run(&rename("new", Some(rename("Untitled", Some(redo))))).unwrap_err();
        assert!(
            err.contains("output of its redo answers to other.cmd"),
            "{err}"
        );
    }

    #[test]
    fn an_inverse_or_redo_without_events_is_refused() {
        let mut inverse = rename("Untitled", Some(rename("new", None)));
        if let Fake::Rename { events, .. } = &mut inverse {
            *events = false;
        }
        let err = run(&rename("new", Some(inverse))).unwrap_err();
        assert!(
            err.contains("its inverse fake.cmd reports no event"),
            "{err}"
        );
        let mut redo = rename("new", None);
        if let Fake::Rename { events, .. } = &mut redo {
            *events = false;
        }
        let err = run(&rename("new", Some(rename("Untitled", Some(redo))))).unwrap_err();
        assert!(err.contains("redo fake.cmd reports no event"), "{err}");
    }

    #[test]
    fn a_wrong_inverse_is_refused() {
        let wrong = rename(
            "new",
            Some(rename("Not the original", Some(rename("new", None)))),
        );
        assert!(
            run(&wrong)
                .unwrap_err()
                .contains("does not restore the state")
        );
    }

    #[test]
    fn an_inverse_that_is_not_an_edit_is_refused() {
        let live_inverse = Fake::Rename {
            to: "Untitled".into(),
            kind: Kind::Live,
            undo: UndoKind::None,
            inverse: None,
            events: false,
            answers: "fake.cmd",
            destructive: false,
        };
        let fake = rename("new", Some(live_inverse));
        assert!(run(&fake).unwrap_err().contains("is not an edit"));
    }

    #[test]
    fn an_inverse_that_fails_or_lacks_a_redo_is_refused() {
        let fake = rename("new", Some(Fake::Refuse { taint: false }));
        assert!(run(&fake).unwrap_err().contains("fails"));
        let fake = rename("new", Some(rename("Untitled", None)));
        assert!(run(&fake).unwrap_err().contains("no redo"));
    }

    #[test]
    fn a_redo_that_does_not_bring_the_edit_back_is_refused() {
        let fake = rename(
            "new",
            Some(rename("Untitled", Some(rename("elsewhere", None)))),
        );
        assert!(
            run(&fake)
                .unwrap_err()
                .contains("does not bring the edit back")
        );
    }

    #[test]
    fn a_refusal_must_leave_the_state_untouched() {
        check_refusal(&Fake::Refuse { taint: false }, &state(), &context()).unwrap();
        let err = check_refusal(&Fake::Refuse { taint: true }, &state(), &context()).unwrap_err();
        assert!(err.contains("but the state changed"));
        let err = check_refusal(&good(), &state(), &context()).unwrap_err();
        assert!(err.contains("expected a refusal"));
    }
}
