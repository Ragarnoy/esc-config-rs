# Revamp and audit

Goal: turn esc-config-rs into a set of crates that a flight controller can embed,
starting with Elle (proposal `elle/docs/changes/0002-esc-config-over-rpc.md`), while the
CLI + relay keep working. This file is the plan; tick items off as they land. The
target layout is a proposal, not a decision: the audit (Phase 1) may change it.

## Where things stand (2026-10-02)

- `cargo test` passes: 150 unit + 2 integration tests in `esc-protocol`, 28 in
  `rtttl-rs`, 5 in `esc-relay-core`, plus doc tests. The CLI has none.
- `esc-protocol` and `esc-relay-core` build for `thumbv8m.main-none-eabihf`.
- A large uncommitted change set sits in the working tree (37 files; `esc-dshot`,
  `beep_test`, `motor_test`, `dshot_pio.rs` deleted). **Phase 0 commits it.**

### Structural problems

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
- [ ] Commit the working tree as it is (it builds and tests pass), so the revamp diffs
      against something.
- [ ] `cargo clippy --all-targets` clean (8 warnings now), `cargo fmt`.
- [ ] CI: test, clippy, `no_std` build of the `no_std` crates for thumbv8m, firmware
      build.

### Phase 1: audit (read-only, produces findings)
- [ ] Every protocol claim in `docs/` and `CLAUDE.md` checked against the code and,
      where possible, against AM32's bootloader source. Mismatches listed above get a
      verdict.
- [ ] Every public item: used? tested? Dead code listed for removal.
- [ ] Error handling: `unwrap`/`expect`/silent truncation (`DataPacket::new` clamps to
      280 bytes without telling anyone) listed.
- [ ] Bench: bootloader entry (high vs low, power cycle or signal-timeout reset), the
      real inter-command timeouts, keepalive interval. Record in
      `docs/protocol-direct.md`.
- [ ] Output: a findings list in this file, each with a fix or a "won't fix".

### Phase 2: core extraction
- [ ] `Link` trait + `esc-session` + fake ESC tests.
- [ ] Move the sequencing out of `cli/src/relay.rs` into `esc-session`.
- [ ] `am32_config` no_std.
- [ ] Delete `esc-relay-core` (or rename it to `esc-session` if that is cleaner).

### Phase 3: firmware and CLI on the new core
- [ ] `esc-onewire-rp` crate, clock-derived divider.
- [ ] `esc-config-icd`; relay firmware speaks it over USB (postcard + COBS, like Elle).
- [ ] CLI talks the ICD; byte passthrough mode kept only if am32.ca direct-connect
      still matters (it relies on the FTDI VID trick).
- [ ] Bench regression: backup / set / restore / flash on a real ESC, compared with
      backups taken before the revamp.

### Phase 4: hand-off to Elle
- [ ] Tag a release; Elle pins it by git rev.
- [ ] Elle proposal 0002 updated with the final crate names and `Link` shape.
