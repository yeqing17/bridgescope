use std::collections::{BTreeMap, HashMap, HashSet};

use fadb_adb::AdbTransport;
use fadb_domain::{
    BridgeError, DeviceDescriptor, DeviceOverview, DeviceRecord, DeviceSerial, DeviceSnapshot,
    DeviceTarget, ErrorCode,
};

#[derive(Debug, Default)]
pub struct DeviceRegistry {
    records: BTreeMap<DeviceSerial, DeviceRecord>,
    generations: HashMap<DeviceSerial, u64>,
    selected: Option<DeviceSerial>,
}

impl DeviceRegistry {
    pub fn reconcile(&mut self, devices: Vec<DeviceDescriptor>) -> DeviceSnapshot {
        let previous_serials = self.records.keys().cloned().collect::<HashSet<_>>();
        let incoming_serials = devices
            .iter()
            .map(|device| device.serial.clone())
            .collect::<HashSet<_>>();

        for removed in previous_serials.difference(&incoming_serials) {
            self.generations
                .entry(removed.clone())
                .and_modify(|generation| *generation = generation.saturating_add(1))
                .or_insert(1);
        }

        let mut records = BTreeMap::new();
        for descriptor in devices {
            let serial = descriptor.serial.clone();
            let generation = match self.records.get(&serial) {
                Some(previous) if same_transport(&previous.descriptor, &descriptor) => {
                    previous.generation
                }
                Some(previous) => previous.generation.saturating_add(1),
                None => self
                    .generations
                    .get(&serial)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1),
            };
            self.generations.insert(serial.clone(), generation);
            records.insert(
                serial,
                DeviceRecord {
                    descriptor,
                    generation,
                },
            );
        }
        self.records = records;

        if self
            .selected
            .as_ref()
            .is_some_and(|serial| !self.records.contains_key(serial))
        {
            self.selected = None;
        }
        self.snapshot()
    }

    pub fn select(&mut self, serial: Option<DeviceSerial>) -> Result<DeviceSnapshot, BridgeError> {
        if let Some(serial) = &serial
            && !self.records.contains_key(serial)
        {
            return Err(BridgeError::new(
                ErrorCode::DeviceNotFound,
                "device.not_found",
                serial.redacted(),
            ));
        }
        self.selected = serial;
        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> DeviceSnapshot {
        DeviceSnapshot {
            devices: self.records.values().cloned().collect(),
            selected: self.selected.clone(),
        }
    }

    pub fn selected_record(&self) -> Option<&DeviceRecord> {
        self.selected
            .as_ref()
            .and_then(|serial| self.records.get(serial))
    }

    /// Returns a target only while the selected device is currently online.
    #[must_use]
    pub fn current_online_target(&self) -> Option<DeviceTarget> {
        self.selected_record()
            .filter(|record| record.descriptor.state.is_online())
            .map(DeviceRecord::target)
    }

    /// Resolves a target only if its connection generation is still online.
    #[must_use]
    pub fn current_online(&self, target: &DeviceTarget) -> Option<&DeviceRecord> {
        self.records.get(&target.serial).filter(|record| {
            record.generation == target.generation && record.descriptor.state.is_online()
        })
    }

    pub fn generation(&self, serial: &DeviceSerial) -> Option<u64> {
        self.records.get(serial).map(|record| record.generation)
    }
}

fn same_transport(previous: &DeviceDescriptor, current: &DeviceDescriptor) -> bool {
    previous.state == current.state && previous.transport_id == current.transport_id
}

pub async fn load_overview(
    transport: &dyn AdbTransport,
    serial: &DeviceSerial,
    expected_generation: u64,
    registry: &DeviceRegistry,
) -> Result<DeviceOverview, BridgeError> {
    if registry.generation(serial) != Some(expected_generation) {
        return Err(BridgeError::new(
            ErrorCode::DeviceUnavailable,
            "device.generation_changed",
            serial.redacted(),
        ));
    }
    transport.device_overview(serial).await
}

#[cfg(test)]
mod tests {
    use fadb_domain::DeviceState;

    use super::*;

    fn device(serial: &str, state: DeviceState) -> DeviceDescriptor {
        device_with_transport(serial, state, None)
    }

    fn device_with_transport(
        serial: &str,
        state: DeviceState,
        transport_id: Option<u64>,
    ) -> DeviceDescriptor {
        DeviceDescriptor {
            serial: DeviceSerial::new(serial).expect("valid serial"),
            state,
            product: None,
            model: None,
            device: None,
            transport_id,
        }
    }

    #[test]
    fn never_selects_first_device_implicitly() {
        let mut registry = DeviceRegistry::default();
        let snapshot = registry.reconcile(vec![
            device("one", DeviceState::Online),
            device("two", DeviceState::Online),
        ]);
        assert_eq!(snapshot.selected, None);
        assert_eq!(registry.current_online_target(), None);
    }

    #[test]
    fn selection_is_cleared_when_device_detaches() {
        let mut registry = DeviceRegistry::default();
        registry.reconcile(vec![device("one", DeviceState::Online)]);
        registry
            .select(Some(DeviceSerial::new("one").expect("valid serial")))
            .expect("device exists");
        let snapshot = registry.reconcile(Vec::new());
        assert_eq!(snapshot.selected, None);
        assert_eq!(registry.current_online_target(), None);
    }

    #[test]
    fn generation_changes_after_state_transition_and_reconnect() {
        let mut registry = DeviceRegistry::default();
        let serial = DeviceSerial::new("one").expect("valid serial");
        registry.reconcile(vec![device("one", DeviceState::Online)]);
        let first = registry.generation(&serial).expect("generation");
        registry.reconcile(vec![device("one", DeviceState::Offline)]);
        let second = registry.generation(&serial).expect("generation");
        registry.reconcile(Vec::new());
        registry.reconcile(vec![device("one", DeviceState::Online)]);
        let third = registry.generation(&serial).expect("generation");
        assert!(first < second && second < third);
    }

    #[test]
    fn generation_changes_when_transport_id_changes() {
        let mut registry = DeviceRegistry::default();
        let serial = DeviceSerial::new("one").expect("valid serial");
        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(11),
        )]);
        let first = registry.generation(&serial).expect("generation");

        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(12),
        )]);
        let second = registry.generation(&serial).expect("generation");

        assert_eq!(second, first + 1);
    }

    #[test]
    fn generation_is_stable_for_same_transport_and_metadata_changes() {
        let mut registry = DeviceRegistry::default();
        let serial = DeviceSerial::new("one").expect("valid serial");
        let mut first = device_with_transport("one", DeviceState::Online, Some(11));
        first.model = Some("old".to_owned());
        registry.reconcile(vec![first]);
        let generation = registry.generation(&serial).expect("generation");

        let mut updated = device_with_transport("one", DeviceState::Online, Some(11));
        updated.model = Some("new".to_owned());
        registry.reconcile(vec![updated]);

        assert_eq!(registry.generation(&serial), Some(generation));
    }

    #[test]
    fn transport_id_presence_changes_generation_conservatively() {
        let mut registry = DeviceRegistry::default();
        let serial = DeviceSerial::new("one").expect("valid serial");
        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(11),
        )]);
        let first = registry.generation(&serial).expect("generation");

        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            None,
        )]);
        let missing = registry.generation(&serial).expect("generation");

        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(12),
        )]);
        let replacement = registry.generation(&serial).expect("generation");

        assert!(first < missing && missing < replacement);
    }

    #[test]
    fn selected_current_online_target_requires_online_state() {
        let mut registry = DeviceRegistry::default();
        let serial = DeviceSerial::new("one").expect("valid serial");
        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Offline,
            Some(1),
        )]);
        registry
            .select(Some(serial.clone()))
            .expect("device exists even while offline");
        assert_eq!(registry.current_online_target(), None);

        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(1),
        )]);
        let target = registry.current_online_target().expect("online target");
        assert_eq!(target.serial, serial);
        assert_eq!(
            registry.current_online(&target).map(DeviceRecord::target),
            Some(target)
        );
    }

    #[test]
    fn stale_target_does_not_resolve_after_reconnect() {
        let mut registry = DeviceRegistry::default();
        let serial = DeviceSerial::new("one").expect("valid serial");
        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(1),
        )]);
        registry
            .select(Some(serial))
            .expect("device exists and can be selected");
        let stale = registry.current_online_target().expect("online target");

        registry.reconcile(vec![device_with_transport(
            "one",
            DeviceState::Online,
            Some(2),
        )]);

        assert_eq!(registry.current_online(&stale), None);
        assert_ne!(registry.current_online_target(), Some(stale));
    }

    #[test]
    fn refuses_unknown_selection() {
        let mut registry = DeviceRegistry::default();
        let result = registry.select(Some(DeviceSerial::new("missing").expect("valid serial")));
        assert_eq!(
            result.expect_err("must fail").code,
            ErrorCode::DeviceNotFound
        );
    }
}
