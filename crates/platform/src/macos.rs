//! macOS drive listing via `diskutil -plist`.

// The parser is compiled everywhere for tests but only called on macos.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::collections::{HashMap, HashSet};

use plist::{Dictionary, Value};

use crate::{Drive, Error, Result};

/// Lists every physical whole disk except the boot disk.
#[cfg(target_os = "macos")]
pub fn list() -> Result<Vec<Drive>> {
    let disks = parse_plist(&crate::run("diskutil", &["list", "-plist", "physical"])?)?;
    let apfs = parse_plist(&crate::run("diskutil", &["apfs", "list", "-plist"])?)?;
    let root = parse_plist(&crate::run("diskutil", &["info", "-plist", "/"])?)?;

    let whole = whole_disks(&disks);
    let mut infos = HashMap::new();
    for id in &whole {
        let info = parse_plist(&crate::run("diskutil", &["info", "-plist", id])?)?;
        infos.insert(id.clone(), info);
    }
    Ok(build(&disks, &apfs, &root, &infos))
}

pub(crate) fn parse_plist(bytes: &[u8]) -> Result<Dictionary> {
    Value::from_reader(std::io::Cursor::new(bytes))
        .map_err(|e| Error::Parse {
            what: "diskutil output",
            detail: e.to_string(),
        })?
        .into_dictionary()
        .ok_or(Error::Parse {
            what: "diskutil output",
            detail: "not a dictionary".into(),
        })
}

fn whole_disks(list: &Dictionary) -> Vec<String> {
    list.get("WholeDisks")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_string)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn str_of<'a>(d: &'a Dictionary, key: &str) -> Option<&'a str> {
    d.get(key).and_then(Value::as_string)
}

fn bool_of(d: &Dictionary, key: &str) -> bool {
    d.get(key).and_then(Value::as_boolean).unwrap_or(false)
}

fn u64_of(d: &Dictionary, key: &str) -> Option<u64> {
    d.get(key).and_then(Value::as_unsigned_integer)
}

/// "disk4s2" → "disk4". Synthesized APFS disks are separate whole disks, so
/// this only strips the partition suffix.
fn whole_of(id: &str) -> &str {
    match id.strip_prefix("disk").and_then(|rest| rest.find('s')) {
        Some(i) => &id[..4 + i],
        None => id,
    }
}

/// Physical disks backing the container that holds `/`.
fn system_disks(root: &Dictionary) -> HashSet<String> {
    let mut out = HashSet::new();
    if let Some(stores) = root.get("APFSPhysicalStores").and_then(Value::as_array) {
        for s in stores.iter().filter_map(Value::as_dictionary) {
            if let Some(id) = str_of(s, "APFSPhysicalStore") {
                out.insert(whole_of(id).to_string());
            }
        }
    }
    // Non-APFS root (very old macOS): the parent disk itself.
    if out.is_empty() {
        if let Some(id) = str_of(root, "ParentWholeDisk") {
            out.insert(id.to_string());
        }
    }
    out
}

/// APFS volume names keyed by the physical whole disk that stores them.
fn apfs_volumes(apfs: &Dictionary) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let containers = apfs.get("Containers").and_then(Value::as_array);
    for c in containers.into_iter().flatten().filter_map(Value::as_dictionary) {
        let names: Vec<String> = c
            .get("Volumes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_dictionary)
            .filter_map(|v| str_of(v, "Name"))
            .map(String::from)
            .collect();
        let stores = c.get("PhysicalStores").and_then(Value::as_array);
        for s in stores.into_iter().flatten().filter_map(Value::as_dictionary) {
            if let Some(id) = str_of(s, "DeviceIdentifier") {
                out.entry(whole_of(id).to_string())
                    .or_default()
                    .extend(names.iter().cloned());
            }
        }
    }
    out
}

/// Plain (non-APFS) partition volume names, skipping the hidden EFI partition.
fn partition_volumes(list: &Dictionary) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let all = list.get("AllDisksAndPartitions").and_then(Value::as_array);
    for disk in all.into_iter().flatten().filter_map(Value::as_dictionary) {
        let Some(id) = str_of(disk, "DeviceIdentifier") else {
            continue;
        };
        let names = out.entry(id.to_string()).or_default();
        // A disk formatted without a partition table has its volume on the disk itself.
        if let Some(n) = str_of(disk, "VolumeName") {
            names.push(n.to_string());
        }
        let parts = disk.get("Partitions").and_then(Value::as_array);
        for p in parts.into_iter().flatten().filter_map(Value::as_dictionary) {
            if str_of(p, "Content") == Some("EFI") {
                continue;
            }
            if let Some(n) = str_of(p, "VolumeName").filter(|n| !n.is_empty()) {
                names.push(n.to_string());
            }
        }
    }
    out
}

fn bus_label(protocol: Option<&str>) -> Option<String> {
    let label = match protocol? {
        "USB" => "USB",
        "Secure Digital" => "SD card",
        "Thunderbolt" => "Thunderbolt",
        "PCI-Express" | "PCI" => "PCIe",
        "SATA" => "SATA",
        "Apple Fabric" => "Internal",
        other => other,
    };
    Some(label.to_string())
}

fn display_name(info: &Dictionary) -> String {
    let reg = str_of(info, "IORegistryEntryName").map(|n| n.trim_end_matches(" Media"));
    let name = crate::join_name(&[reg]);
    if !name.is_empty() {
        return name;
    }
    let media = crate::join_name(&[str_of(info, "MediaName")]);
    if media.is_empty() {
        "Unnamed drive".into()
    } else {
        media
    }
}

pub(crate) fn build(
    list: &Dictionary,
    apfs: &Dictionary,
    root: &Dictionary,
    infos: &HashMap<String, Dictionary>,
) -> Vec<Drive> {
    let system = system_disks(root);
    let apfs_vols = apfs_volumes(apfs);
    let part_vols = partition_volumes(list);

    let mut drives = Vec::new();
    for id in whole_disks(list) {
        if system.contains(&id) {
            continue;
        }
        let Some(info) = infos.get(&id) else { continue };
        if str_of(info, "VirtualOrPhysical") == Some("Virtual") {
            continue;
        }
        let protocol = str_of(info, "BusProtocol");
        let removable = !bool_of(info, "Internal")
            && (matches!(protocol, Some("USB" | "Secure Digital"))
                || bool_of(info, "RemovableMediaOrExternalDevice"));
        let mut volumes = part_vols.get(&id).cloned().unwrap_or_default();
        volumes.extend(apfs_vols.get(&id).cloned().unwrap_or_default());
        volumes.dedup();
        drives.push(Drive {
            name: display_name(info),
            size: u64_of(info, "TotalSize")
                .or_else(|| u64_of(info, "Size"))
                .unwrap_or(0),
            bus: bus_label(protocol),
            removable,
            volumes,
            id,
        });
    }
    drives
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(json: &str) -> Dictionary {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        plist::to_value(&v).unwrap().into_dictionary().unwrap()
    }

    #[test]
    fn whole_of_strips_partition() {
        assert_eq!(whole_of("disk4s2"), "disk4");
        assert_eq!(whole_of("disk12s1"), "disk12");
        assert_eq!(whole_of("disk3"), "disk3");
    }

    #[test]
    fn builds_from_real_shapes() {
        // Trimmed from an M-series Mac with one external USB SSD plugged in.
        let list = dict(
            r#"{
            "WholeDisks": ["disk0", "disk4"],
            "AllDisksAndPartitions": [
                {"DeviceIdentifier": "disk0", "Partitions": [
                    {"DeviceIdentifier": "disk0s2", "Content": "Apple_APFS"}]},
                {"DeviceIdentifier": "disk4", "Partitions": [
                    {"DeviceIdentifier": "disk4s1", "Content": "EFI", "VolumeName": "EFI"},
                    {"DeviceIdentifier": "disk4s2", "Content": "Apple_APFS"}]}
            ]}"#,
        );
        let apfs = dict(
            r#"{"Containers": [
            {"PhysicalStores": [{"DeviceIdentifier": "disk0s2"}],
             "Volumes": [{"Name": "Macintosh HD"}, {"Name": "Data"}]},
            {"PhysicalStores": [{"DeviceIdentifier": "disk4s2"}],
             "Volumes": [{"Name": "MyDrive"}]}
        ]}"#,
        );
        let root = dict(r#"{"APFSPhysicalStores": [{"APFSPhysicalStore": "disk0s2"}]}"#);
        let mut infos = HashMap::new();
        infos.insert(
            "disk0".into(),
            dict(r#"{"Internal": true, "BusProtocol": "Apple Fabric", "TotalSize": 251000193024, "IORegistryEntryName": "APPLE SSD Media"}"#),
        );
        infos.insert(
            "disk4".into(),
            dict(r#"{"Internal": false, "BusProtocol": "USB", "TotalSize": 1000204886016,
                     "IORegistryEntryName": "CT1000P3 PSSD8 Media", "VirtualOrPhysical": "Physical",
                     "RemovableMediaOrExternalDevice": true}"#),
        );

        let drives = build(&list, &apfs, &root, &infos);
        assert_eq!(
            drives,
            vec![Drive {
                id: "disk4".into(),
                name: "CT1000P3 PSSD8".into(),
                size: 1000204886016,
                bus: Some("USB".into()),
                removable: true,
                volumes: vec!["MyDrive".into()],
            }]
        );
    }

    #[test]
    fn internal_non_boot_disk_is_listed_but_not_removable() {
        let list = dict(r#"{"WholeDisks": ["disk2"], "AllDisksAndPartitions": []}"#);
        let root = dict(r#"{"APFSPhysicalStores": [{"APFSPhysicalStore": "disk0s2"}]}"#);
        let mut infos = HashMap::new();
        infos.insert(
            "disk2".into(),
            dict(r#"{"Internal": true, "BusProtocol": "PCI-Express", "Size": 2000}"#),
        );
        let drives = build(&list, &dict("{}"), &root, &infos);
        assert_eq!(drives.len(), 1);
        assert!(!drives[0].removable);
        assert_eq!(drives[0].bus.as_deref(), Some("PCIe"));
        assert_eq!(drives[0].name, "Unnamed drive");
    }
}
