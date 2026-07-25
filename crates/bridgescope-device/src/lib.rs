use std::collections::{BTreeMap, HashMap, HashSet};

use bridgescope_adb::AdbTransport;
use bridgescope_domain::{
    BridgeError, DeviceDescriptor, DeviceOverview, DeviceRecord, DeviceSerial, DeviceSnapshot,
    ErrorCode,
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
                Some(previous) if previous.descriptor.state == descriptor.state => {
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

    pub fn generation(&self, serial: &DeviceSerial) -> Option<u64> {
        self.records.get(serial).map(|record| record.generation)
    }
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
    use bridgescope_domain::DeviceState;

    use super::*;

    fn device(serial: &str, state: DeviceState) -> DeviceDescriptor {
        DeviceDescriptor {
            serial: DeviceSerial::new(serial).expect("valid serial"),
            state,
            product: None,
            model: None,
            device: None,
            transport_id: None,
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
    fn refuses_unknown_selection() {
        let mut registry = DeviceRegistry::default();
        let result = registry.select(Some(DeviceSerial::new("missing").expect("valid serial")));
        assert_eq!(
            result.expect_err("must fail").code,
            ErrorCode::DeviceNotFound
        );
    }
}
