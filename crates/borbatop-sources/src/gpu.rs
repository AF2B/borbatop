//! Graphics cards: DRM devices under sysfs, plus `nvidia-smi` for NVIDIA readings.

use std::fs;
use std::path::Path;

use borbatop_core::{
    Bytes, GpuCard, GpuVendor, MilliCelsius, parse_millicelsius, parse_nvidia_smi, parse_pci_id,
    uevent_value,
};

use crate::command::run;
use crate::error::CommandError;

const DRM_ROOT: &str = "/sys/class/drm";
const CARD_PREFIX: &str = "card";
const DEVICE_DIR: &str = "device";
const HWMON_DIR: &str = "hwmon";
const TEMPERATURE_FILE: &str = "temp1_input";
const NVIDIA_QUERY: &str =
    "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu";
const NVIDIA_FORMAT: &str = "--format=csv,noheader,nounits";

/// The graphics cards found, and why `nvidia-smi` gave no readings when it did not.
#[derive(Debug)]
pub struct GpuReport {
    cards: Vec<GpuCard>,
    nvidia_error: Option<CommandError>,
}

impl GpuReport {
    /// One entry per card.
    #[must_use]
    pub fn cards(&self) -> &[GpuCard] {
        &self.cards
    }

    /// Why NVIDIA readings are missing: `nvidia-smi` is installed but failed. Absent when the
    /// tool is simply not installed.
    #[must_use]
    pub const fn nvidia_error(&self) -> Option<&CommandError> {
        self.nvidia_error.as_ref()
    }
}

/// Finds the graphics cards and reads what each one reports.
///
/// Missing hardware or tools are not errors: the report is simply shorter.
#[must_use]
pub fn collect_gpus() -> GpuReport {
    collect_gpus_in(Path::new(DRM_ROOT))
}

fn collect_gpus_in(drm_root: &Path) -> GpuReport {
    let (smi_cards, nvidia_error) = match run("nvidia-smi", &[NVIDIA_QUERY, NVIDIA_FORMAT], &[]) {
        Ok(text) => (parse_nvidia_smi(&text), None),
        Err(CommandError::Spawn { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            (Vec::new(), None)
        }
        Err(error) => (Vec::new(), Some(error)),
    };

    let mut cards: Vec<GpuCard> = sysfs_cards(drm_root)
        .into_iter()
        // nvidia-smi knows more than sysfs about NVIDIA cards: prefer it when it answered.
        .filter(|card| smi_cards.is_empty() || card.vendor != GpuVendor::Nvidia)
        .collect();
    cards.extend(smi_cards);
    GpuReport {
        cards,
        nvidia_error,
    }
}

fn sysfs_cards(drm_root: &Path) -> Vec<GpuCard> {
    let Ok(listing) = fs::read_dir(drm_root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = listing
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_card_name(name))
        .collect();
    names.sort();

    names
        .iter()
        .filter_map(|name| read_card(&drm_root.join(name).join(DEVICE_DIR), name))
        .collect()
}

/// `card0`, `card1`, ...: not the connectors (`card0-DP-1`) nor the render nodes.
fn is_card_name(name: &str) -> bool {
    name.strip_prefix(CARD_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
}

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn read_number(path: &Path) -> Option<u64> {
    read_text(path)?.trim().parse().ok()
}

fn read_card(device: &Path, card_name: &str) -> Option<GpuCard> {
    let vendor = GpuVendor::from_pci_id(parse_pci_id(&read_text(&device.join("vendor"))?)?);
    let uevent = read_text(&device.join("uevent")).unwrap_or_default();

    Some(GpuCard {
        name: uevent_value(&uevent, "PCI_SLOT_NAME")
            .unwrap_or(card_name)
            .to_owned(),
        vendor,
        driver: uevent_value(&uevent, "DRIVER").map(str::to_owned),
        utilization_percent: read_number(&device.join("gpu_busy_percent"))
            .and_then(|percent| u32::try_from(percent).ok()),
        memory_used: read_number(&device.join("mem_info_vram_used")).map(Bytes::new),
        memory_total: read_number(&device.join("mem_info_vram_total")).map(Bytes::new),
        temperature: read_temperature(device),
    })
}

fn read_temperature(device: &Path) -> Option<MilliCelsius> {
    let monitors = fs::read_dir(device.join(HWMON_DIR)).ok()?;
    monitors.flatten().find_map(|monitor| {
        let text = read_text(&monitor.path().join(TEMPERATURE_FILE))?;
        parse_millicelsius(&text).ok()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn only_card_devices_count_not_connectors_or_render_nodes() {
        assert!(is_card_name("card0"));
        assert!(is_card_name("card12"));
        for name in ["card", "card0-DP-1", "renderD128", "version", "cardx"] {
            assert!(!is_card_name(name), "{name}");
        }
    }

    #[test]
    fn a_fake_amd_card_is_read_from_sysfs_files() {
        let root = std::env::temp_dir().join(format!("borbatop-gpu-{}", std::process::id()));
        let device = root.join("card0").join("device");
        fs::create_dir_all(device.join("hwmon").join("hwmon3")).unwrap();
        fs::write(device.join("vendor"), "0x1002\n").unwrap();
        fs::write(
            device.join("uevent"),
            "DRIVER=amdgpu\nPCI_SLOT_NAME=0000:03:00.0\n",
        )
        .unwrap();
        fs::write(device.join("gpu_busy_percent"), "37\n").unwrap();
        fs::write(device.join("mem_info_vram_used"), "1048576\n").unwrap();
        fs::write(device.join("mem_info_vram_total"), "8589934592\n").unwrap();
        fs::write(
            device.join("hwmon").join("hwmon3").join("temp1_input"),
            "61000\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("card0-DP-1")).unwrap();

        let cards = sysfs_cards(&root);

        let [card] = cards.as_slice() else {
            fs::remove_dir_all(&root).unwrap();
            return assert_eq!(cards.len(), 1);
        };
        assert_eq!(card.vendor, GpuVendor::Amd);
        assert_eq!(card.driver.as_deref(), Some("amdgpu"));
        assert_eq!(card.name, "0000:03:00.0");
        assert_eq!(card.utilization_percent, Some(37));
        assert_eq!(card.memory_used.unwrap().get(), 1_048_576);
        assert_eq!(card.temperature.unwrap().rounded_degrees(), 61);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_machine_without_drm_has_no_cards_and_no_error() {
        let report = collect_gpus_in(Path::new("/nonexistent-drm"));
        // nvidia-smi may exist on the host running the tests; only the sysfs part is fake.
        assert!(
            report
                .cards()
                .iter()
                .all(|card| card.vendor == GpuVendor::Nvidia)
        );
    }
}
