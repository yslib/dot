use dot::job::{JobId, JobKind, JobSelection, JobSelector};
use dot::schema::Identifier;

fn id(value: &str) -> Identifier {
    Identifier::new(value).expect("test identifier should be valid")
}

#[test]
fn job_identity_is_scoped_by_kind() {
    let package = JobId::Package(id("shared"));
    let action = JobId::Action(id("shared"));
    let link = JobId::Link(id("shared"));

    assert_ne!(package, action);
    assert_ne!(action, link);
    assert_eq!(package.kind(), JobKind::Package);
    assert_eq!(package.name(), "shared");
}

#[test]
fn exact_selection_keeps_its_typed_selector() {
    let selection = JobSelection::only(JobSelector::Package(id("cli-tools")));

    assert!(matches!(
        selection,
        JobSelection::Only(ref selectors)
            if selectors.contains(&JobSelector::Package(id("cli-tools")))
    ));
}
