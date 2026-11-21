//! Graphics cards: PCI vendor identifiers, sysfs text and `nvidia-smi` output.

use crate::bytes::Bytes;
use crate::memory::MilliCelsius;

const PCI_ID_PREFIX: &str = "0x";
const HEX_RADIX: u32 = 16;
const VENDOR_NVIDIA: u16 = 0x10de;
const VENDOR_AMD: u16 = 0x1002;
const VENDOR_INTEL: u16 = 0x8086;
const CSV_SEPARATOR: char = ',';
const NVIDIA_FIELDS: usize = 5;
const KIB_PER_MIB: u64 = 1_024;
const MILLI_PER_DEGREE: i32 = 1_000;
const NOT_AVAILABLE_MARK: char = '[';
const UEVENT_SEPARATOR: char = '=';
const NVIDIA_DRIVER: &str = "nvidia";

/// Who makes a graphics card, from its PCI vendor identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpuVendor {
    /// NVIDIA (PCI vendor `0x10de`).
    Nvidia,
    /// AMD / ATI (PCI vendor `0x1002`).
    Amd,
    /// Intel (PCI vendor `0x8086`).
    Intel,
    /// Any other vendor, with its PCI identifier.
    Other(u16),
}

impl GpuVendor {
    /// The vendor for a PCI vendor identifier.
    ///
    /// # Arguments
    ///
    /// * `id` - the 16-bit PCI vendor identifier
    #[must_use]
    pub const fn from_pci_id(id: u16) -> Self {
        match id {
            VENDOR_NVIDIA => Self::Nvidia,
            VENDOR_AMD => Self::Amd,
            VENDOR_INTEL => Self::Intel,
            other => Self::Other(other),
        }
    }
}

/// What is known about one graphics card; every reading may be missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCard {
    /// Marketing name when known, otherwise the PCI address.
    pub name: String,
    /// Who makes it.
    pub vendor: GpuVendor,
    /// Kernel driver bound to it.
    pub driver: Option<String>,
    /// Share of time the card was busy, in percent.
    pub utilization_percent: Option<u32>,
    /// Video memory in use.
    pub memory_used: Option<Bytes>,
    /// Video memory installed.
    pub memory_total: Option<Bytes>,
    /// Core temperature.
    pub temperature: Option<MilliCelsius>,
}

/// Parses a PCI identifier file such as `/sys/class/drm/card0/device/vendor` (`0x10de`).
///
/// # Arguments
///
/// * `text` - contents of the file
#[must_use]
pub fn parse_pci_id(text: &str) -> Option<u16> {
    let digits = text.trim().strip_prefix(PCI_ID_PREFIX)?;
    u16::from_str_radix(digits, HEX_RADIX).ok()
}

/// The value of `key` in `uevent` text (`DRIVER=amdgpu`).
///
/// # Arguments
///
/// * `text` - contents of a `uevent` file
/// * `key` - variable name, without the `=`
#[must_use]
pub fn uevent_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(UEVENT_SEPARATOR)?;
        (name == key).then_some(value)
    })
}

/// Parses `nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu
/// --format=csv,noheader,nounits`; rows that do not have five fields are left out and a field the
/// card cannot report (`[N/A]`) becomes `None`.
///
/// # Arguments
///
/// * `csv` - standard output of the command
#[must_use]
pub fn parse_nvidia_smi(csv: &str) -> Vec<GpuCard> {
    csv.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(CSV_SEPARATOR).map(str::trim).collect();
            let [name, utilization, used, total, temperature] = fields.as_slice() else {
                return None;
            };
            debug_assert_eq!(fields.len(), NVIDIA_FIELDS);
            Some(GpuCard {
                name: (*name).to_owned(),
                vendor: GpuVendor::Nvidia,
                driver: Some(NVIDIA_DRIVER.to_owned()),
                utilization_percent: number(utilization),
                memory_used: mebibytes(used),
                memory_total: mebibytes(total),
                temperature: number(temperature).and_then(|degrees| {
                    i32::try_from(degrees)
                        .ok()?
                        .checked_mul(MILLI_PER_DEGREE)
                        .map(MilliCelsius::new)
                }),
            })
        })
        .collect()
}

fn number(field: &str) -> Option<u32> {
    if field.starts_with(NOT_AVAILABLE_MARK) {
        return None;
    }
    field.parse().ok()
}

fn mebibytes(field: &str) -> Option<Bytes> {
    let mib: u64 = number(field).map(u64::from)?;
    Bytes::from_kib(mib.checked_mul(KIB_PER_MIB)?)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn pci_vendors_map_to_names() {
        assert_eq!(GpuVendor::from_pci_id(0x10de), GpuVendor::Nvidia);
        assert_eq!(GpuVendor::from_pci_id(0x1002), GpuVendor::Amd);
        assert_eq!(GpuVendor::from_pci_id(0x8086), GpuVendor::Intel);
        assert_eq!(GpuVendor::from_pci_id(0x1234), GpuVendor::Other(0x1234));
    }

    #[test]
    fn pci_ids_parse_with_or_without_a_newline_and_reject_junk() {
        assert_eq!(parse_pci_id("0x10de\n"), Some(0x10de));
        assert_eq!(parse_pci_id("0x0001"), Some(1));
        for text in ["", "10de", "0x", "0xzzzz", "0x12345"] {
            assert_eq!(parse_pci_id(text), None, "{text:?}");
        }
    }

    #[test]
    fn uevent_values_are_found_by_exact_key() {
        let text = "DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_SLOT_NAME=0000:03:00.0\nMODALIAS=pci:x\n";
        assert_eq!(uevent_value(text, "DRIVER"), Some("amdgpu"));
        assert_eq!(uevent_value(text, "PCI_SLOT_NAME"), Some("0000:03:00.0"));
        assert_eq!(uevent_value(text, "DRIV"), None);
        assert_eq!(uevent_value("", "DRIVER"), None);
    }

    #[test]
    fn nvidia_smi_rows_become_cards() {
        let cards = parse_nvidia_smi(
            "NVIDIA GeForce RTX 3060, 12, 1024, 12288, 45\nNVIDIA T400, [N/A], [N/A], 4096, [N/A]\nbroken, row\n",
        );
        let [first, second] = cards.as_slice() else {
            return assert_eq!(cards.len(), 2);
        };
        assert_eq!(first.name, "NVIDIA GeForce RTX 3060");
        assert_eq!(first.utilization_percent, Some(12));
        assert_eq!(first.memory_used.unwrap().get(), 1_024 * 1_048_576);
        assert_eq!(first.memory_total.unwrap().get(), 12_288 * 1_048_576);
        assert_eq!(first.temperature.unwrap().rounded_degrees(), 45);
        assert_eq!(second.utilization_percent, None);
        assert_eq!(second.memory_used, None);
        assert_eq!(second.temperature, None);
        assert_eq!(second.vendor, GpuVendor::Nvidia);
    }
}
