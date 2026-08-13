use muxtrix_domain::{
    DomainError, LaunchProfile, PaneTree, ProcessBackend, ProfileId, SessionState, SplitAxis,
    SplitRatio, Surface, TerminalSurface, Workspace,
};

fn terminal(profile_id: ProfileId, title: &str) -> Surface {
    Surface::terminal(
        title,
        TerminalSurface {
            profile_id,
            working_directory: None,
        },
    )
}

#[test]
fn nested_split_focus_close_and_persistence_flow() -> Result<(), Box<dyn std::error::Error>> {
    let profile = LaunchProfile {
        id: ProfileId::new(),
        name: "integration shell".into(),
        backend: ProcessBackend::Local,
        program: "/bin/sh".into(),
        arguments: vec!["-l".into()],
        working_directory: None,
    };
    let mut workspace = Workspace::new("integration", terminal(profile.id, "one"));
    let first = workspace
        .active_tab()
        .expect("new workspace should have an active tab")
        .focused_pane_id;
    let second = workspace.split_focused(
        SplitAxis::Horizontal,
        SplitRatio::EQUAL,
        terminal(profile.id, "two"),
    )?;
    let third = workspace.split_focused(
        SplitAxis::Vertical,
        SplitRatio::new(600)?,
        terminal(profile.id, "three"),
    )?;

    workspace.validate()?;
    let tab = workspace
        .active_tab()
        .expect("active tab should remain available");
    assert_eq!(tab.root.pane_ids(), vec![first, second, third]);
    assert_eq!(tab.focused_pane_id, third);
    assert!(matches!(tab.root, PaneTree::Split { .. }));

    workspace
        .active_tab_mut()
        .expect("active tab should remain available")
        .focused_pane_id = first;
    workspace.close_pane(second)?;
    workspace.validate()?;
    let tab = workspace
        .active_tab()
        .expect("active tab should remain available");
    assert_eq!(tab.root.pane_ids(), vec![first, third]);
    assert_eq!(tab.focused_pane_id, first);

    let state = SessionState::new(workspace, vec![profile]);
    let json = serde_json::to_string_pretty(&state)?;
    let restored: SessionState = serde_json::from_str(&json)?;
    restored.validate()?;
    assert_eq!(restored, state);
    Ok(())
}

#[test]
fn closing_the_only_pane_is_rejected_without_mutation() {
    let profile_id = ProfileId::new();
    let mut workspace = Workspace::new("integration", terminal(profile_id, "only"));
    let pane_id = workspace
        .active_tab()
        .expect("new workspace should have an active tab")
        .focused_pane_id;

    assert_eq!(
        workspace.close_pane(pane_id),
        Err(DomainError::CannotCloseLastPane)
    );
    let tab = workspace
        .active_tab()
        .expect("active tab should remain available");
    assert_eq!(tab.root.pane_ids(), vec![pane_id]);
    assert_eq!(tab.panes.len(), 1);
}
