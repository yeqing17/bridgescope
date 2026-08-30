use fadb_domain::OperationRisk;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationAction {
    List,
    Install,
    Launch,
    ForceStop,
    Uninstall,
    ClearData,
    Enable,
    Disable,
}

impl ApplicationAction {
    pub fn risk(self) -> OperationRisk {
        match self {
            Self::List => OperationRisk::ReadOnly,
            Self::Install | Self::Launch | Self::ForceStop | Self::Enable | Self::Disable => {
                OperationRisk::Mutating
            }
            Self::Uninstall | Self::ClearData => OperationRisk::Destructive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destructive_actions_are_classified_in_backend() {
        assert_eq!(
            ApplicationAction::Uninstall.risk(),
            OperationRisk::Destructive
        );
        assert_eq!(
            ApplicationAction::ClearData.risk(),
            OperationRisk::Destructive
        );
    }
}
