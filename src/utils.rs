pub fn check_project_compliance(params: &tower_lsp::lsp_types::InitializeParams) -> bool {
    if let Some(root_uri) = params.root_uri.as_ref() {
        let root_path = root_uri.to_file_path().unwrap();

        return root_path.join("SpaceStation14.sln").exists() || root_path.join("RobustToolbox/RobustToolbox.sln").exists();
    }

    false
}