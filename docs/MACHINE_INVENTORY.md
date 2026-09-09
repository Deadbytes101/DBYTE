# Read-only machine inventory (v13 Generation 0)

Status: DRAFT. Schema `dbyte-machine-inventory-v1`. These tools collect OS evidence;
they do not test a DByte kernel or select/authorize a disk.

## Running on the target

Copy the Windows script, or the Linux script **and** its adjacent Python helper,
to the spare PC. Choose based on its actual OS. No remote connection, network,
installation or elevation is attempted. Default output is JSON stdout; diagnostics
go to stderr. Scripts do not create output files, mount media, change firmware,
install drivers, read raw block devices or issue disk writes. OS background activity
and writes caused by the user's chosen shell redirection are outside this guarantee.

Windows (PowerShell 3+ with ConvertTo-Json; CIM or WMI fallback):

```powershell
powershell.exe -NoProfile -File .\collect_machine_inventory.ps1
```

Linux (POSIX shell and Python 3.6+ already installed):

```sh
sh ./collect_machine_inventory.sh
```

Python is a host-side JSON/probe helper, not kernel runtime. No Python installation
is attempted. Without it, Linux exits 2 with INVENTORY_DEPENDENCY_UNAVAILABLE. lspci
and lsblk are optional and missing/failed commands remain UNAVAILABLE. PowerShell
2 cannot emit this JSON schema; use an already available suitable OS/toolchain.

To save a receipt, explicitly redirect stdout to a user-selected file on a suitable
medium. Keep stderr separate. Receipts contain serials/UUIDs; return them privately
or review before publishing. Do not redact identifiers and then treat the receipt
as full identity evidence. No collector is run on the main Windows machine as a
substitute for the spare PC.

## Interpretation

- Each observation carries status, value and source. OBSERVED means the named OS
  interface supplied it, not independent hardware verification. UNKNOWN means missing
  or inconclusive; UNAVAILABLE means a query/tool/capability could not be used.
- Direct CPUID is not executed. Linux exposes OS-reported flags/family/model; Windows
  exposes WMI processor identity/width, with raw CPUID flags explicitly unavailable.
- Current UEFI boot is evidence of UEFI support. Legacy boot or missing EFI sysfs is
  not evidence of BIOS-only firmware. Containers/VMs expose their presented machine,
  not the physical host's complete topology.
- PCI class_code includes class/subclass/prog-if. Driver/service binding and controller
  links are evidence; storage_active_mode remains UNKNOWN rather than inventing a
  firmware SATA setting. Linux device/sysfs paths and Windows association edges must
  be retained during review. No BAR sizing writes are used.
- Linux mount roles follow major:minor through partitions and slave graphs; a RAID
  volume can identify multiple leaves. Missing edges mark the result incomplete.
  Unmounted ESPs, firmware boot disks and other namespaces remain unproven. Windows
  MSFT_Disk IsBoot/IsSystem and WMI partition/OS associations provide complementary
  evidence; older OSes may lack the modern storage namespace.
- Linux records event device key capabilities and bus/path evidence. Windows follows
  keyboard PnP parents up to 16 edges. HID alone does not imply USB. Empty i8042
  observations do not prove controller absence.
- GPU data is OS adapter evidence, not a GOP framebuffer handoff receipt.

Fingerprint is SHA-256 of the included UTF-8 preimage. Core identity requires CPU
vendor/model and board vendor/model plus at least one independently observed stable
discriminator: baseboard serial or system UUID. Legacy boards that genuinely expose
no baseboard serial may therefore remain complete when a meaningful system UUID is
observed; if both discriminators are missing or placeholder values, completeness is
false. Timestamps and disk numbering are excluded. Reordering devices does not change
identity. Windows/Linux serialization and OS model strings may differ; cross-OS
comparisons inspect identity fields, not just hashes. Disk identity is recorded
separately and must be revalidated even if the machine fingerprint matches.

Every receipt starts with all selections UNSELECTED, disk_write NO WRITE and real
hardware/full OS PENDING. It cannot grant write permission. Fixture receipts contain
`fixture: true` and are never real-machine acceptance evidence.

## Tests and next action

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts\verify_machine_inventory.ps1
```

The wrapper runs both the collector fixture suite and the receipt-validation suite. A returned target receipt can then be checked without echoing its serial/UUID identity fields:

```powershell
python -B scripts\verify_machine_inventory_receipt.py <target-receipt.json>
```

Exit 0 means the receipt is structurally valid, non-fixture, fingerprint/identity-consistent, still carries the collector's fail-closed `NO WRITE` acceptance state, and has observed long mode, PCI topology, a storage-class controller, an input transport, and disk identity needed for target-selection review. Exit 2 means structurally valid but incomplete; inspect `blockers` and obtain more read-only evidence. Exit 1 means malformed, fingerprint-mismatched, unsupported, or acceptance state was altered. The verifier never selects hardware and always reports `write_authorized: false`.

Saved fixtures are synthetic OS observation extracts, not receipts from the spare
PC. Tests cover missing tools/permission failure, legacy ambiguity, unknown input,
multi-disk backing, changed identity, incomplete graphs and JSON/fingerprint output.
Windows fixture execution requires existing PowerShell; a skipped Windows test is
not proof of Windows support. Linux wrapper syntax and fixture execution should also
be checked on Linux/WSL. These tests do not certify older Windows or real hardware.

Return an unmodified target receipt to complete machine selection. If required fields
are unavailable, obtain additional read-only evidence on that machine; do not infer
them from the ECS/Phenom labels. Follow [the hardware acceptance contract](V13_MACHINE_FOUNDATION.md#hardware-acceptance-and-write-authorization).
