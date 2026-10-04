//! Linux drive listing via `lsblk --json`.
//!
//! The system disk is found by walking each disk's tree of partitions, LUKS
//! mappings and LVM volumes for the root, boot or swap mounts, so a root on
//! LVM-on-LUKS still marks the physical disk underneath it.

// The parser is compiled everywhere for tests but only called on linux.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use serde::Deserialize;

use crate::{Drive, Error, Result};

#[cfg(target_os = "linux")]
pub fn list() -> Result<Vec<Drive>> {
    const COLS: &str = "NAME,PATH,SIZE,MODEL,VENDOR,TRAN,RM,HOTPLUG,TYPE,LABEL";
    // MOUNTPOINTS (util-linux 2.37+) lists every mount, e.g. btrfs subvolumes at / and /home.
    // Older lsblk rejects the column, so fall back to the single MOUNTPOINT.
    let out = crate::run(
        "lsblk",
        &[
            "--json",
            "--bytes",
            "--output",
            &format!("{COLS},MOUNTPOINTS"),
        ],
    )
    .or_else(|_| {
        crate::run(
            "lsblk",
            &[
                "--json",
                "--bytes",
                "--output",
                &format!("{COLS},MOUNTPOINT"),
            ],
        )
    })?;
    parse(&out)
}

/// Mount points that mean "the running system lives on this disk".
const SYSTEM_MOUNTS: &[&str] = &["/", "/boot", "/boot/efi", "/efi", "/usr", "/var", "[SWAP]"];

#[derive(Deserialize)]
struct Lsblk {
    blockdevices: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    name: String,
    path: Option<String>,
    size: Option<Num>,
    model: Option<String>,
    vendor: Option<String>,
    tran: Option<String>,
    rm: Option<Flag>,
    hotplug: Option<Flag>,
    #[serde(rename = "type")]
    kind: Option<String>,
    label: Option<String>,
    mountpoint: Option<String>,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Node>,
}

/// Old util-linux prints numbers and booleans as strings ("1", "0").
#[derive(Deserialize)]
#[serde(untagged)]
enum Num {
    Int(u64),
    Str(String),
}

impl Num {
    fn get(&self) -> Option<u64> {
        match self {
            Num::Int(n) => Some(*n),
            Num::Str(s) => s.parse().ok(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Str(String),
}

impl Flag {
    fn get(&self) -> bool {
        match self {
            Flag::Bool(b) => *b,
            Flag::Str(s) => s == "1" || s == "true",
        }
    }
}

impl Node {
    fn mounts(&self) -> impl Iterator<Item = &str> {
        self.mountpoint
            .as_deref()
            .into_iter()
            .chain(self.mountpoints.iter().flatten().map(String::as_str))
    }

    fn hosts_system(&self) -> bool {
        self.mounts().any(|m| SYSTEM_MOUNTS.contains(&m))
            || self.children.iter().any(Node::hosts_system)
    }

    fn collect_volumes(&self, out: &mut Vec<String>) {
        let label = self.label.as_deref().filter(|l| !l.is_empty());
        let mount = self
            .mounts()
            .find(|m| m.starts_with('/'))
            .and_then(|m| m.rsplit('/').next())
            .filter(|m| !m.is_empty());
        if let Some(name) = label.or(mount) {
            if !out.iter().any(|v| v == name) {
                out.push(name.to_string());
            }
        }
        for c in &self.children {
            c.collect_volumes(out);
        }
    }
}

fn bus_label(tran: Option<&str>) -> Option<String> {
    let label = match tran? {
        "usb" => "USB",
        "mmc" => "SD card",
        "nvme" => "NVMe",
        "sata" | "ata" => "SATA",
        "scsi" | "sas" => "SCSI",
        other => other,
    };
    Some(label.to_string())
}

pub(crate) fn parse(json: &[u8]) -> Result<Vec<Drive>> {
    let tree: Lsblk = serde_json::from_slice(json).map_err(|e| Error::Parse {
        what: "lsblk output",
        detail: e.to_string(),
    })?;
    let mut drives = Vec::new();
    for disk in tree.blockdevices {
        if disk.kind.as_deref() != Some("disk") || disk.hosts_system() {
            continue;
        }
        // zram swap devices and the like show up as disks with no transport and no size.
        if disk.name.starts_with("zram") {
            continue;
        }
        let size = disk.size.as_ref().and_then(Num::get).unwrap_or(0);
        if size == 0 {
            continue;
        }
        let tran = disk.tran.as_deref();
        let removable = matches!(tran, Some("usb" | "mmc"))
            || disk.rm.as_ref().is_some_and(Flag::get)
            || disk.hotplug.as_ref().is_some_and(Flag::get);
        let mut name = crate::join_name(&[disk.vendor.as_deref(), disk.model.as_deref()]);
        if name.is_empty() {
            name = if tran == Some("mmc") {
                "SD card".into()
            } else {
                "Unnamed drive".into()
            };
        }
        let mut volumes = Vec::new();
        for c in &disk.children {
            c.collect_volumes(&mut volumes);
        }
        drives.push(Drive {
            id: disk.path.unwrap_or_else(|| format!("/dev/{}", disk.name)),
            name,
            size,
            bus: bus_label(tran),
            removable,
            volumes,
        });
    }
    Ok(drives)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_system_disk_behind_lvm_on_luks() {
        let json = br#"{"blockdevices": [
          {"name":"nvme0n1","path":"/dev/nvme0n1","size":512110190592,"model":"Samsung SSD 980","vendor":null,
           "tran":"nvme","rm":false,"hotplug":false,"type":"disk","label":null,"mountpoint":null,
           "children":[
             {"name":"nvme0n1p1","type":"part","label":"EFI","mountpoint":"/boot/efi"},
             {"name":"nvme0n1p2","type":"part","mountpoint":null,"children":[
               {"name":"luks-abc","type":"crypt","mountpoint":null,"children":[
                 {"name":"vg-root","type":"lvm","mountpoint":"/"}]}]}]},
          {"name":"sdb","path":"/dev/sdb","size":"32010928128","model":"Ultra           ","vendor":"SanDisk ",
           "tran":"usb","rm":"1","hotplug":"1","type":"disk","label":null,"mountpoint":null,
           "children":[{"name":"sdb1","type":"part","label":"UNTITLED","mountpoint":"/media/u/UNTITLED"}]},
          {"name":"loop0","path":"/dev/loop0","size":4096,"type":"loop","mountpoint":"/snap/core/1"},
          {"name":"zram0","path":"/dev/zram0","size":8589934592,"type":"disk","mountpoint":"[SWAP]"},
          {"name":"mmcblk0","path":"/dev/mmcblk0","size":63864569856,"model":null,"vendor":null,
           "tran":"mmc","rm":false,"hotplug":false,"type":"disk","children":[]}
        ]}"#;
        let drives = parse(json).unwrap();
        let ids: Vec<_> = drives.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["/dev/sdb", "/dev/mmcblk0"]);

        let sd = &drives[0];
        assert_eq!(sd.name, "SanDisk Ultra");
        assert_eq!(sd.size, 32010928128);
        assert_eq!(sd.bus.as_deref(), Some("USB"));
        assert!(sd.removable);
        assert_eq!(sd.volumes, ["UNTITLED"]);

        let mmc = &drives[1];
        assert_eq!(mmc.name, "SD card");
        assert!(mmc.removable);
    }

    #[test]
    fn internal_data_disk_listed_as_non_removable() {
        let json = br#"{"blockdevices": [
          {"name":"sda","size":2000398934016,"model":"WDC WD20","tran":"sata","rm":false,"hotplug":false,
           "type":"disk","children":[{"name":"sda1","type":"part","label":"Media","mountpoints":["/mnt/media"]}]},
          {"name":"sdc","size":1000204886016,"model":"Btrfs Disk","tran":"sata","rm":false,"type":"disk",
           "children":[{"name":"sdc1","type":"part","mountpoints":["/home","/"]}]}]}"#;
        let drives = parse(json).unwrap();
        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].id, "/dev/sda");
        assert!(!drives[0].removable);
        assert_eq!(drives[0].volumes, ["Media"]);
    }
}
