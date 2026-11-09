# Revamp and audit

Goal: turn esc-config-rs into a set of crates that a flight controller can embed,
starting with Elle (proposal `elle/docs/changes/0002-esc-config-over-rpc.md`). The CLI
and relay may be broken during Phase 2 and must work again at the end of Phase 3.
This file is the plan; tick items off as they land. The
target layout is a proposal, not a decision: the audit (Phase 1) may change it.

## Where things stand (2026-10-02, Phase 3 code done, bench pending)

- `esc-session` holds the bootloader driver; 30 tests run it against a simulated AM32
  bootloader. `esc-protocol` is reduced to AM32 frames, classification, layout,
  settings layout, melody and Intel HEX, all `no_std`. `esc-relay-core`, MSP and
  four-way are gone.
- The relay firmware runs `esc-session` over `esc-onewire-rp` and serves
  `esc-config-icd` endpoints; the CLI calls them. Nothing has run on hardware yet:
  the bench regression and the Phase 1 bench checklist are what is left.
- Below, "Structural problems" is the pre-revamp state (Phase 0 baseline); all six
  are resolved (1, 3, 4, 6 in Phase 2; 2, 5 in Phase 3).

### Structural problems (baseline)

1. **The logic that talks to an ESC lives in the CLI.** `esc-configurator-cli/src/relay.rs`
   holds connect, read, write, commit, keepalive, read-modify-write of the EEPROM block,
   flashing. It is blocking, `std`, and tied to `serialport`. An FC cannot reuse it,
   and it has no tests.
2. **The relay firmware guesses where commands start and end.** The PC drives the bootloader over USB, so `esc-relay-fw`
   pattern-matches the byte stream (`tx_buf[0] == 0xFF && tx_buf[6] == 0xFE …`) and
   waits 2 ms to reassemble USB fragments, to keep AM32's no-ACK Set Buffer → data
   sequence atomic.
3. **`esc-relay-core` is dead.** Nothing depends on it; its `Relay` state machine and
   command protocol are not what the firmware runs.
4. **Duplication.** `esc-relay-fw` has its own `crc16_ibm` and does not use
   `esc-protocol` at all.
5. **`OneWireUart` hardcodes a 150 MHz system clock.** Elle runs at 200 MHz, so the
   UART would run at ~25.6 kbaud. The divider must come from `clk_sys_freq()`.
6. **`am32_config` is `std`-only for no real reason** (the only `std` use found is a
   `ToString` in tests). An FC wants the field layout without `std`.

### Doc and code inconsistencies found so far

- Bootloader entry: docs say "power-on with signal held **low**"; the usual AM32
  behaviour is that a line **driven high** keeps the bootloader. Settle on the bench
  (this also blocks Elle's proposal).
- ESC pin: `esc-relay-fw` module doc says GPIO0, code uses GPIO15.
- Set Buffer "gets no ACK" everywhere, but `Relay::write_256` sends a 3-byte header
  and waits for an ACK (0x30/0x31). Either dead code or the docs are wrong.
- Set Buffer is documented as `FE 00 00 SIZE` (README, protocol-direct.md) and
  `FE 00 x256 LEN` (relay-fw doc).
- Timeouts differ between `protocol-direct.md` (25 / 50 / 75 ms) and the firmware
  (1 ms gaps, 100 / 200 / 500 ms). One table, measured, should win.

Verdicts are in the Phase 1 findings below (P1–P6).

## Phase 1 findings

Checked against the AM32 bootloader (`am32-firmware/AM32-bootloader` @ `578ff29`,
2026-09-17, `bootloader/main.c`, `Mcu/*/Src/eeprom.c`) and firmware
(`am32-firmware/AM32` @ `2738df3`, `Inc/eeprom.h`, `Src/main.c`). Source-derived;
items marked **bench** still need confirming on hardware. Severity: **S1** can leave
an ESC unbootable or silently lose data, **S2** wrong behaviour on some hardware,
**S3** cleanup.

### How the bootloader actually behaves

These facts drive the `esc-session` design.

- **Entry.** At reset it samples the signal pin: with a pull-down, if the line is
  mostly low it jumps to the app (unless this was a software reset); with a pull-up,
  if it was never low it **stays**; if it floats and reads low once, it jumps. So a
  line **driven high** at power-on keeps the bootloader. After that, a line held low
  for 20 ms while waiting for a byte also jumps to the app.
- **No inactivity timeout.** With the line idle high the bootloader waits for a start
  bit forever. It jumps only when `invalid_command > 100` (bad CRC, bad command,
  framing errors) or on a low line. DroneCAN builds resume CAN polling after ~10 s of
  silence. **Keepalive is not needed** on a direct link; it exists for 4-way
  passthrough.
- **Framing is by gaps.** A message ends when no start bit comes within 5 bit times
  of the previous byte's mid-stop-bit (~230 µs idle at 19200). Bytes of one frame
  must be back to back; two frames need ≥ ~300 µs between them.
- **Set Buffer** `FE 00 HI LO`: size is `256` if byte 2 is `0x01`, else byte 3. So
  256 is `FE 00 01 00`, and `FE 00 00 00` means **zero** bytes. No reply on success;
  `0xC2` on a bad CRC. There is **no time limit** between Set Buffer and the payload.
- **Payload**: `size + 2` bytes; ACK `0x30`, or `0xC2` on bad CRC, in which case the
  bootloader **stays in payload mode**: the next frame (even a Set Address) is taken
  as payload. Recovery is to resend a well-formed payload of the same size.
- **A command frame is a valid payload.** `receiveBuffer` zero-fills `rxBuffer`
  and `decodeInput` checks the CRC over the announced `size`; a frame that ends in
  its own CRC (every command frame but connect) followed by zeros has CRC 0, so in
  payload mode it is **ACKed as the payload**. A Set Address sent in payload mode
  is swallowed with `0x30`: the next read or write goes to the old address. The
  session uses this: a Set Address always ends payload mode.
- **Prog flash** `01 xx CRC`: writes the buffer at the address. Erases the page
  **only if the address is page-aligned** (F051/F421 1 KiB, G071 2 KiB). Address and
  length must be multiples of 2 (F051), 4 (F421) or 8 (G071). Writes to the EEPROM
  base get byte 2 overwritten with the bootloader version. Refuses addresses below
  the app start (`0xC1`).
- **Erase** `02 xx`: only checks the CRC and ACKs. It erases nothing.
- **Read** `03 N`: `N = 0` means 256. Reply is `DATA CRC_L CRC_H 0x30`, no echo of
  the command. The address resets after each read, so every read needs a Set Address.
- **Set Address**: 16-bit, shifted left 2 on 128 KiB parts (G071). Values below 1024
  are refused except magics: `0x20` EEPROM base, `0x21` file name, `0x22` continue
  after last read, `0x23` devinfo (protocol v3: magic, deviceInfo, address shift,
  firmware/filename/EEPROM/tune start).
- **Run** `00 00 00 00` (CRC of `00 00` is `0000`) makes it jump; it still replies
  `0xC1`. **Keepalive** `FD 00` replies `0xC1`. Anything else replies `0xC1`.
- **Connect** reply is 9 bytes: `'4' '7' '1' PIN FLASH 06 06 PROTO 30`. `PIN` is
  `port << 4 | pin` (PA2 `02`, PA6 `06`, PB4 `14`, PA15 `0F`, PA0 `00`, PB2 `12`, PC6
  `26`); `FLASH` is `1F` 32K, `35` 64K, `2B` 128K (NXP: `15`, `16`).
- **Boot gate**: the bootloader jumps only if EEPROM byte 0 is `0x01` and the app's
  vector table looks sane.
- **EEPROM** (`EEprom_t`) is 192 bytes: config 0–47, tune 48–175, CAN 176–183,
  reserved 184–191. The firmware saves all 192. `EEPROM_VERSION` is 4.

### Verdicts on the inconsistencies above

- **P1 Bootloader entry: driven high.** "Power-on with signal held low" in CLAUDE.md
  and README is wrong. Corollary: the RP2350 pad default is pull-down,
  so a relay that resets or loses power makes the ESC leave the bootloader after
  20 ms (if byte 0 is `0x01`). Bench: confirm both.
- **P2 ESC pin: GPIO15** (code). Module doc is stale.
- **P3 Set Buffer ACK: none on success.** `Relay::write_256` (3-byte `FE 00 00`, then
  waits for an ACK) is dead and wrong; delete it.
- **P4 Set Buffer format: `FE 00 HI LO`**. The relay-fw doc's "x256" is the right idea;
  README/`protocol-direct.md` "SIZE=0 means 256" is wrong, and so is the firmware's
  `tx_buf[9] == 0 → 256`. `packet.rs` has it backwards: `OP_WRITE_256` (`FE 00 01 00`)
  is the AM32 form, and `build_write_page_packet` (`FE 00 CRC`) is not valid for AM32.
- **P5 Timeouts.** Neither table is right; the bootloader has no command timeouts.
  Derived numbers to confirm on the bench:

  | Step | Expected reply | Suggested timeout |
  |---|---|---|
  | Connect | 9 bytes, ~5 ms | 100 ms, retry |
  | Set Address | `30`, < 1 ms after frame end | 20 ms |
  | Set Buffer | none; send payload ≥ 0.3 ms later | n/a |
  | Payload ≤ 256 | `30`, ~1 ms | 20 ms |
  | Prog flash | `30`, erase + program + verify | 100 ms (bench G071 2 KiB erase) |
  | Read N | N+3 bytes, ~0.52 ms each (256: ~135 ms) | 50 ms + 0.6 ms × (N+3) |
  | Within a frame | inter-byte gap | < 200 µs (hard limit ~230 µs) |

- **P6 "ESC rejects 256-byte writes (NACK 0xC1)"** is almost certainly the size
  encoding: `256 as u8 = 0` sends `FE 00 00 00`, a zero-length buffer. The
  bootloader's receive buffer is 258 bytes; `FE 00 01 00` should work. Bench.

### Findings

| # | Sev | Where | Finding | Fix |
|---|---|---|---|---|
| F1 | S1 | `am32_config.rs` `with_defaults`, `reset-defaults` | Writes byte 0 = `0x00`. The bootloader then never jumps to the app: `reset-defaults` leaves the ESC stuck in the bootloader. The comment "byte 0 is reserved now" is wrong; `flash_firmware` relies on it as the boot gate. Also sets `eeprom_version = 2`, so the firmware replaces bytes 5–11 with its own defaults on next boot. | Byte 0 = `0x01`; keep the ESC's version bytes (RMW); never write version < 4. |
| F2 | S1 | `relay.rs` `write_eeprom_block` | Writes 176 bytes at the page-aligned EEPROM base, so the page is erased and CAN settings (176–183) are lost on every save. | Write the full 192-byte struct (multiple of 8). |
| F3 | S1 | `bootloader.rs` `validate_read_response`; `read_32/48/128` | Read CRC is never checked (TODO). Backups, verify and RMW can carry corrupted bytes, and RMW writes them back. | Check CRC on every read; retry. |
| F4 | S1 | `commands.rs` `restore_config` | Restores only bytes 0–47 of a 256-byte backup: melody and CAN are not restored. No check that the backup matches the ESC. | Restore 192 bytes; refuse mismatched layout/version. |
| F5 | S2 | `commands.rs` flash + verify | `firmware_start = 0x1000` and no address shift. G071 (`2B`): writes to real `0x4000`, so the app lands in the wrong place and the ESC stays in the bootloader. DroneCAN bootloaders (app at `0x4000`) refuse the writes. u16 arithmetic (`binary.len() as u16`, `0x1000 + offset`) wraps for > 60 KiB images. | Take firmware start, EEPROM, file name and shift from devinfo (magic `0x23`, protocol v3), fall back to the flash-size code. Address the hex's own load address. |
| F6 | S2 | `relay.rs` `read_esc_name`, `read_melody` | `eeprom - 32` and `eeprom + 48` in protocol units: on G071 these are 128 / 192 bytes off. Writes are unaffected (whole block). | Use magics `0x21` and `0x20` + offsets, or devinfo. |
| F7 | S2 | `device.rs` | ID table lists `1F02 1F06 3306 3406 3506 2B06 4706`: flash codes `33 34 47` do not exist in AM32, and pins PB4/PA15/PA0/PB2/PC6 are rejected as unknown. `might_be_blheli32` (low byte `06` = "ARM-ish") is meaningless: `06` is pin PA6. `ensure_supported` panics. | Classify AM32 by `"471"` + `06 06` + flash code; read devinfo for the rest. Errors, not panics. |
| F8 | S2 | `flash_firmware` retry loop | After a `0xC2` the bootloader stays in payload mode, so each retry's Set Address is consumed as payload; the 8 retries cannot recover. | On `0xC2`, resend the payload; model this in the fake ESC. |
| F9 | S2 | `ihex::to_binary`, flash | Last chunk is not padded: odd / non-multiple-of-8 lengths are refused (`0xC1`) on F051/F421/G071. | Pad with `0xFF` to 8 bytes. |
| F10 | S2 | `relay-fw` `onewire_uart::write` | Each `write()` ends with a 530 µs wait and the next starts with 100 µs, so payload and CRC sent by two `write()` calls are ~110–170 µs apart, against a ~230 µs frame limit. Works today, little margin. | One `write()` per frame. In `esc-session`: `Link::send` = one frame. |
| F11 | S2 | `relay-fw` set-buffer path | Host sending Set Buffer and payload in one USB write is forwarded as one burst; the bootloader then treats the payload as part of the Set Buffer frame and the next frame as payload. Matters for am32.ca direct-connect. | Goes away with typed operations (Phase 3). |
| F12 | S2 | `commands.rs` `read_config` (`read` command) | Reads 16 bytes at `0xF7AC` (`DEVICE_ID_ADDRESS`, a BLHeli_S/SiLabs address). On 32 KiB parts that is past the end of flash; at best garbage, at worst a bus fault in the bootloader. Same for `TIMESTAMP_ADDRESS 0xEB00`. | Remove; show devinfo instead. |
| F13 | S2 | `disconnect` | Sends `00 01`, which the bootloader answers `0xC1` and ignores; the ESC stays in the bootloader. "The ESC will restart with the new firmware" is false. | `Run` = `00 00 00 00`; say so in the CLI. |
| F14 | S2 | BLHeli_S | `backup`, `restore`, `read` do not check firmware type and use AM32 addresses (`0x7C00`) on SiLabs parts; BLHeli_S uses a different EEPROM location and write flow. Nothing BLHeli_S is exercised on hardware. | Mark unsupported; refuse non-AM32 everywhere. |
| F15 | S3 | relay-fw | Aggregation loop drops a USB packet that would overflow `tx_buf` (noted in Phase 0). `DataPacket::new` silently clamps to 280 bytes. | Gone with Phase 3; until then, error out. |
| F16 | S3 | `relay.rs` | `write_chunk` sends `size as u8` (256 → 0); `read_flash` silently truncates `size` to 256; `read_esc_name` maps invalid UTF-8 to `""`. | Typed sizes in `esc-session`. |
| F17 | S3 | `flash_firmware` | Page-erase loop sends `0x02` per page: a no-op (the bootloader erases on aligned writes), with a hardcoded 1 KiB page. | Drop. |
| F18 | S3 | dead code | `Relay::{keepalive, write_256, erase_page, read_timestamp}`; `packet::{OP_WRITE_PAGE, build_write_page_packet, build_erase_page_packet, ADDR_*}`; `esc-relay-core` (whole crate); 10 `#[allow(dead_code)]` constants in `am32_config.rs`. | Delete in Phase 2. |
| F19 | S3 | `am32_config.rs` | Byte 13 is `brake_on_zero_throttle` in current AM32, treated as reserved here (RMW preserves it). `ConfigBlock` and CLI output call the block "encrypted" (BLHeli_S wording). `reset-defaults` mentions the deleted `beep_test`/`motor_test`. | Add the field; fix wording. |
| F20 | S3 | panics | `.expect("should be connected")` ×15 in `commands.rs` (all after a successful `connect`, so safe); `from_utf8(..).unwrap()` in `melody_info` (guarded by `is_rtttl_format`); `spawn(..).unwrap()` in firmware `main` (fine). | Keep; F7 removes the real panics. |
| F21 | S3 | `esc-protocol` tests | `cargo test -p esc-protocol` fails without `std` (Phase 0 note). | `required-features = ["std"]`. |

Unit tests exist for `crc`, `packet`, `bootloader`, `device`, `config`,
`am32_config`, `melody`, `ihex`, `msp`, `fourway*`, `rtttl-rs`. None for `relay.rs`,
`commands.rs` or the firmware, which is where F1–F13 live. The tests that exist
encode several of the errors above (device table, the `FE 00` write-page packet, the
`00 01` disconnect), so they pass while the behaviour is wrong.

### Scope answers

- **MSP + four-way**: the CLI does not use them (no `esc_protocol::msp`/`fourway`
  import), Elle does not need them. **Done:** dropped from `esc-protocol` (9955559);
  git keeps them if Betaflight passthrough comes back.
- **BLHeli_S**: not exercised anywhere. **Done:** unsupported; SiLabs IDs are
  recognised and refused at connect with a named error (F14).
- **Flashing**: F051 (`1F`) and 64 KiB parts (`35`) work on paper; G071 is broken
  (F5); DroneCAN bootloaders are refused. No test covers the shift.

### Bench checklist (needs hardware)

- [ ] Entry: ESC power-on with the line driven high / pulled low / floating;
      relay reset while the ESC is in the bootloader (expect: leaves after 20 ms).
- [ ] Idle: leave a connected ESC 60 s without traffic; expect it still answers
      (no keepalive needed).
- [ ] 256-byte write with `FE 00 01 00` (P6).
- [ ] Timings in P5 with a logic analyser, especially Prog flash on G071 if one is
      available, and the payload→CRC gap from the relay (F10).
- [ ] `00 00 00 00` starts the app; `00 01` does not (F13).
- [ ] Byte 0 = `0x00` keeps the ESC in the bootloader after power cycle (F1). Use
      an ESC with a backup.

## Target layout (proposal)

```
esc-protocol      no_std, no I/O. Frames, CRC, device table, AM32 EEPROM layout
                  (no_std; Display/by-name settings behind `std`), melody, ihex records.
esc-session       no_std, async, no hardware. The bootloader driver: connect, read,
                  write (Set Address → Set Buffer → data → CRC → commit, never split),
                  keepalive, read-modify-write of the 176-byte block, flash plan.
                  Generic over a small `Link` trait. Owns every timing rule.
esc-onewire-rp    RP2040/RP2350 PIO half-duplex UART implementing `Link`. Clock from
                  `clk_sys_freq()`. Shared by esc-relay-fw and Elle.
esc-config-icd    no_std, serde types for the operations: Connect / Read / Write /
                  Disconnect / Flash, their responses and a typed error. Elle's RPC and
                  the relay's USB protocol both carry these.
esc-relay-fw      Thin: USB transport + esc-session + esc-onewire-rp. No byte sniffing.
esc-configurator-cli  std: settings UI, backups (TOML), RTTTL, hex files. Talks the ICD
                  to "a device" (relay over USB now; Elle over RTT later).
rtttl-rs          unchanged
```

`Link` sketch (to be settled in Phase 2):

```rust
pub trait Link {
    type Error;
    /// Send bytes, then release the line for the reply (half duplex).
    async fn send(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
    /// Receive exactly `buf.len()` bytes, or fail after `timeout`.
    async fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> Result<(), Self::Error>;
    async fn delay(&mut self, d: Duration);
}
```

Keep it independent of embassy so host tests can run it; `Duration` from a tiny
local type or `embassy-time` behind a feature.

### Test strategy

- **A simulated AM32 bootloader** (`esc-session/tests/fake_esc.rs`): a `Link` that
  plays the bootloader from a byte-array flash image, including the quirks (no ACK on
  Set Buffer, 256-byte write NACK, page erase on write, keepalive 0xC1). Every session
  operation gets tested against it, including read-modify-write leaving the other half
  of the block intact, and a write cut off mid-way.
- Recorded traces from the real relay (once the bench confirms them) become fixtures.
- CLI keeps its parsing tests; I/O goes through the ICD, mockable.

### Scope questions for the audit

- MSP + four-way (Betaflight passthrough): keep, move to the CLI, or drop? Elle does
  not need them.
- BLHeli_S (SiLabs): is it tested on hardware? If not, mark it unsupported rather than
  carry untested paths.
- Flashing: which MCUs are verified (F051 / G071 / F3)? G071 divides addresses by 4;
  is that covered by a test?

## Phases

### Phase 0: baseline
- [x] Commit the working tree as it is (it builds and tests pass), so the revamp diffs
      against something. (f07519a)
- [x] `cargo clippy --all-targets` clean (8 warnings now), `cargo fmt`.
- [x] CI: test, clippy, `no_std` build of the `no_std` crates for thumbv8m, firmware
      build. (`.github/workflows/ci.yaml`, all with `-D warnings`)
- [x] Embassy pinned to Elle's rev (`340bf1de`, same git URL so the lockfiles agree);
      the firmware now uses `embassy_rp::executor::Executor` (the executor's
      `arch-cortex-m` feature is gone at that rev). `toml` 0.8 → 1; everything else
      at the latest semver-compatible release (`fixed` stays on 1.x, 2.0 is alpha).

Noticed during Phase 0, for the audit:

- `cargo test -p esc-protocol` alone fails: `tests/firmware_integration.rs` uses
  `load_firmware`, which needs `std`. It only passes in `cargo test` because the CLI
  turns `std` on through feature unification. Needs `required-features = ["std"]`.
- Relay firmware, USB aggregation loop (`esc_task`): when a packet would overflow
  `tx_buf`, the loop breaks and that packet is dropped, not forwarded.
- `clippy::pedantic` reports ~500 warnings; most are cosmetic (`must_use`, doc
  backticks), but ~45 are truncating casts (`u16`/`usize` → `u8`) worth checking.
- `rtttl-rs` is still on edition 2021.

### Phase 1: audit (read-only, produces findings)
- [x] Every protocol claim in `docs/` and `CLAUDE.md` checked against the code and,
      where possible, against AM32's bootloader source. Mismatches listed above get a
      verdict. (P1–P6; source-based, bench items open)
- [x] Every public item: used? tested? Dead code listed for removal. (F18)
- [x] Error handling: `unwrap`/`expect`/silent truncation (`DataPacket::new` clamps to
      280 bytes without telling anyone) listed. (F3, F15, F16, F20)
- [ ] Bench: bootloader entry (high vs low, power cycle or signal-timeout reset), the
      real inter-command timeouts, keepalive interval. Record in this file
      (the bench checklist below).
- [x] Output: a findings list in this file, each with a fix or a "won't fix".
      (Phase 1 findings; scope answers need a decision)

### Phase 2: core extraction
- [x] `Link` trait + `esc-session` + fake ESC tests.
- [x] Move the sequencing out of `cli/src/relay.rs` into `esc-session`.
- [x] `am32_config` no_std.
- [x] Delete `esc-relay-core` (or rename it to `esc-session` if that is cleaner).
- [x] Fix the findings in the new core (status below).

`Link` as landed: `send(frame)` (one frame, back to back, line released after),
`recv(buf, timeout) -> usize` (bytes received before the timeout; a timeout is not an
error), `delay(d)`. `Duration` is a local µs type. Retries and timeouts are in a
`Timing` struct with direct-wire defaults; a link with its own latency adds to them
(the CLI's relay link adds 100 ms per reply).

Findings status after Phase 2:

| # | Status |
|---|---|
| F1 | Fixed: `reset-defaults` is read-modify-write (`Am32Config::apply_defaults`: version ≥ 4, keeps boot byte, melody, CAN, versions), so byte 0 stays `0x01`. It does not set byte 0 either: after a failed flash that would boot half an image. |
| F2 | Fixed: `Esc::write_eeprom` writes the 192-byte struct. |
| F3 | Fixed: every read is CRC-checked and retried (`parse_read_reply`). |
| F4 | Fixed: `restore` writes bytes 5..192, keeps boot/version bytes, refuses another EEPROM layout version. Settings edits (`set`, `import`, interactive save) only touch bytes 5..48. |
| F5 | Fixed: `Layout` from devinfo or flash code; flash addresses shifted; image start checked against the hex. |
| F6 | Fixed: file name from the layout; melody from the settings block. |
| F7 | Fixed: classify by flash code; no panics. |
| F8 | Fixed: a payload answered `C2` or not at all is resent. |
| F9 | Fixed: last chunk padded to 8 bytes. |
| F10 | Session side: one `send` per frame. Relay: payload + CRC now one `write()`. |
| F11 | Open until Phase 3 (relay byte sniffing). |
| F12 | Fixed: `read` shows connect info and layout instead of reading `0xF7AC`. |
| F13 | Fixed: `Esc::run` sends `00 00 00 00`; CLI `run` command. Other commands leave the ESC in the bootloader. |
| F14 | Fixed (Phase 1). |
| F15 | Open until Phase 3. |
| F16 | Fixed in the session (typed lengths, `InvalidLength`). |
| F17 | Fixed: no erase commands. |
| F18 | Fixed: old `Session`, `config.rs`, unused frames and `esc-relay-core` deleted. |
| F19 | Partly: offset 13 named; not exposed as a setting yet. "Encrypted" wording gone. |
| F20 | `expect`s gone with the old CLI flow. |
| F21 | Fixed: `required-features = ["std"]`. |

Also changed: a failed flash now leaves the boot byte cleared (the old CLI restored
the previous value, which could boot half an image); the CLI no longer needs tokio
(Phase 3 brings it back for postcard-rpc's host client).
Payload mode after a failure (audit before merge): a lost payload ACK used to make
the session resend the payload, which the bootloader then ran as a command; and a
write that gave up left the bootloader in payload mode, where it ACKs the next Set
Address as a payload (see "A command frame is a valid payload"), so the following
read or write went to the failed write's address. `write_at` now answers a missing
payload ACK with Set Address + Set Buffer, and ends every failed write with a Set
Address. Only a link error mid-write can still leave payload mode set.

### Phase 3: firmware and CLI on the new core
- [x] `esc-onewire-rp` crate, clock-derived divider.
- [x] `esc-config-icd`; relay firmware speaks it over USB (postcard + COBS, like Elle).
- [x] CLI talks the ICD; byte passthrough mode kept only if am32.ca direct-connect
      still matters (it relies on the FTDI VID trick). **Dropped**: the relay speaks
      only the ICD; VID:PID moved from the borrowed FTDI `0403:0001` to pid.codes
      `1209:0001`.
- [ ] Bench regression: backup / set / restore / flash on a real ESC, compared with
      backups taken before the revamp.

As landed:

- **Transport.** USB CDC-ACM; each message is a postcard-rpc header + postcard body,
  COBS-encoded, 0x00-delimited (Elle's RTT framing, `FrameBuf` copied from
  `elle-system`). postcard-rpc's own embassy-usb server targets embassy-usb 0.5; we
  are on 0.6 (Elle's rev), so `WireTx`/`WireRx` are hand-written (`esc-relay-fw/src/wire.rs`).
  Host: postcard-rpc's `cobs-serial` client.
- **Server shape.** The dispatch context owns the UART and, while connected, the
  connect reply and layout; each `async` handler resumes an `Esc` over the borrowed
  UART (`Esc::resume`, `impl Link for &mut L`). No task or channel: one request at
  a time, run to completion.
- **ICD** (`esc-config-icd`, paths `esc/…`, every response `Result<_, EscError>`):

  | Endpoint | Request | Response |
  |---|---|---|
  | `esc/server/info` | `()` | `ServerInfo` (firmware version) |
  | `esc/connect` | `()` | `EscInfo` (ids, protocol version, layout, max image) |
  | `esc/disconnect` | `()` | `()` (forget the session; ESC stays in its bootloader) |
  | `esc/run` | `()` | `()` (Run frame; the app starts if byte 0 is `0x01`) |
  | `esc/eeprom/read` | `()` | 192 bytes |
  | `esc/eeprom/write` | 192 bytes | `()` |
  | `esc/name` | `()` | 32 bytes |
  | `esc/read` | `{address, len ≤ 256}` | bytes |
  | `esc/flash/begin` | `()` | `()` (boot byte cleared) |
  | `esc/flash/chunk` | `{offset, data ≤ 256}` | `()` (written and read back) |
  | `esc/flash/finish` | `()` | `()` (boot byte set) |
  | `esc/flash/read` | `{offset, len ≤ 256}` | bytes |

- **Host policy** lives in the CLI (`device.rs`, tested against an in-memory
  device): edits touch settings bytes 5..48 only, restore merges 5..192 and refuses
  another EEPROM layout version, a flash is begin / 256-byte chunks / finish and
  leaves the boot byte cleared on error.
- Findings closed in Phase 3: F10 (one frame per `send`, no relay re-framing), F11
  and F15 (no byte sniffing, no USB aggregation), structural problems 2 (relay byte
  matching) and 5 (divider from `clk_sys_freq()`, checked at compile time for
  125/150/200 MHz).
- The pre-audit protocol docs (`protocol-direct.md`, `flash-implementation.md`,
  `memory-layout.md`, `features.md`) are deleted; this file replaces them.

Bench regression (props off; take a backup with the pre-revamp CLI, `f07519a`, first):

1. `esc-configurator read -d <port>`: connect info and layout match the ESC (F051:
   app `0x1000`, EEPROM `0x7C00`; G071: shift 2, EEPROM `0x7E00`).
2. `backup`: byte-identical to the pre-revamp backup.
3. `set -s beep-volume -v 3`, `show`, power-cycle (relay powered first), `show`:
   change kept, melody and CAN bytes unchanged (compare with `backup`).
4. `restore` the step 2 backup: `backup` again is identical to it.
5. `flash` the ESC's current hex, `verify`, `run`: the ESC beeps and runs.
6. With a logic analyser on the signal wire: no gap above ~200 µs inside a frame
   (payload + CRC), and reply timings against P5.

### Phase 4: hand-off to Elle
- [ ] Tag a release; Elle pins it by git rev.
- [ ] Elle proposal 0002 updated with the final crate names and `Link` shape.
