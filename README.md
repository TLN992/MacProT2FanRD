# macproT2fans

A Linux fan control daemon for Mac Pro 2019 (and similar T2-based Macs) that manages fan speeds based on hardware sensor readings.

## Overview

This tool provides intelligent, temperature-controlled fan management for Mac Pro 2019 systems running Linux. It reads temperatures from various hardware sensors (CPU, GPU, NVMe, etc.) and dynamically adjusts fan speeds using the Linux hwmon subsystem.

Key features:
- **Adaptive fan curves**: Linear, exponential, or logarithmic speed profiles
- **Multiple sensor support**: Aggregates data from CPU, GPU, and other sensors
- **Degraded mode fallback**: Maintains safe fan speeds if sensors/drivers are unavailable
- **Configurable per-fan settings**: Individual control for each fan
- **Smooth ramp-down**: Gradual fan speed reduction to prevent oscillation

## Requirements

- Linux kernel with hwmon support
- Root access (required for hardware control)
- Mac Pro 2019 or similar T2-based Mac with compatible hardware monitoring

## Installation

### From Cargo

```bash
cargo build --release
sudo cp target/release/macproT2fans /usr/local/bin/
```

### Using Nix/NixOS

Add to your `flake.nix`:

```nix
{
  inputs.macprot2fans.url = "github:your-username/macproT2fans";

  outputs = { self, nixpkgs, macprot2fans }:
    let system = "x86_64-linux"; in {
      nixosConfigurations.your-host = nixpkgs.lib.nixosSystem {
        modules = [
          macprot2fans.nixosModules.macprot2fans
          # your other modules...
        ];
      };
    };
}
```

Then enable the service:

```nix
services.macprot2fans.enable = true;
```

## Usage

### Basic Commands

```bash
# List available sensors
sudo macproT2fans --list-sensors

# List detected fans with current RPM
sudo macproT2fans --list-fans

# Generate configuration file
sudo macproT2fans --generate-config > /etc/macprot2fans.toml

# Run the daemon (uses config from /etc/macprot2fans.toml)
sudo macproT2fans

# Run with custom config path
sudo macproT2fans --config /path/to/config.toml
```

### Interactive Configuration Wizard

Run the wizard to generate a configuration interactively:

```bash
sudo macproT2fans --generate-config
```

The wizard will:
1. Discover all available sensors and fans
2. Prompt for global defaults (temperature thresholds, speed curve)
3. Allow per-fan customization
4. Output a TOML configuration file

For NixOS users, generate Nix format:

```bash
sudo macproT2fans --generate-nix
```

## Configuration

### TOML Format (`/etc/macprot2fans.toml`)

```toml
[defaults]
low_temp = 55              # Temperature at minimum fan speed (°C)
high_temp = 75             # Temperature at maximum fan speed (°C)
speed_curve = "linear"     # "linear", "exponential", or "logarithmic"
sensor_aggregation = "max" # "max", "average", or "min"
ramp_down_rate = 1.0       # °C/sec for gradual fan slowdown

[degraded]
expected_drivers = ["coretemp", "amdgpu"]
initial_percent = 60       # Fan speed % when sensors missing
escalated_percent = 80     # Speed after escalation delay
escalation_delay = 60      # Seconds before increasing speed

# Per-fan configuration (optional, overrides defaults)
[fan.fan0]
low_temp = 50
high_temp = 70
speed_curve = "exponential"
sensors = ["coretemp CPU0", "coretemp CPU1"]
```

### NixOS Format

```nix
{
  services.macprot2fans = {
    enable = true;

    defaults = {
      low_temp = 55;
      high_temp = 75;
      speed_curve = "linear";
      sensor_aggregation = "max";
      ramp_down_rate = 1.0;
    };

    degraded = {
      expected_drivers = [ "coretemp" "amdgpu" ];
      initial_percent = 60;
      escalated_percent = 80;
      escalation_delay = 60;
    };

    fans.fan0 = {
      low_temp = 50;
      high_temp = 70;
      sensors = [ "coretemp CPU0" "coretemp CPU1" ];
    };
  };
}
```

## Configuration Options

### Global Defaults

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `low_temp` | integer | 55 | Temperature at which fans run at minimum speed |
| `high_temp` | integer | 75 | Temperature at which fans run at maximum speed |
| `speed_curve` | string | "linear" | Fan speed curve type (linear/exponential/logarithmic) |
| `sensor_aggregation` | string | "max" | Method to combine multiple sensors (max/average/min) |
| `ramp_down_rate` | float | 1.0 | Rate of fan speed reduction when temperature drops (°C/sec) |

### Degraded Mode

When expected hardware drivers are not available, the daemon enters degraded mode:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `expected_drivers` | list | ["coretemp", "amdgpu"] | Drivers that must be present for normal operation |
| `initial_percent` | integer | 60 | Initial fan speed percentage in degraded mode |
| `escalated_percent` | integer | 80 | Fan speed after escalation delay |
| `escalation_delay` | integer | 60 | Seconds before increasing fan speed |

### Per-Fan Settings

Each fan can override global defaults:

```toml
[fan.fan0]
low_temp = 50              # Override low temp threshold
high_temp = 72             # Override high temp threshold
speed_curve = "exponential" # Custom curve for this fan
sensors = ["coretemp CPU0"] # Specific sensors for this fan (uses all if omitted)
```

## How It Works

### Hardware Discovery

The daemon scans:
- **CPU sensors**: Through `coretemp` driver (Intel/AMD)
- **GPU sensors**: Through `amdgpu` driver (AMD GPUs)
- **Apple SMC**: Via `applesmc` (Mac-specific sensors)
- **NVMe drives**: Temperature monitoring via `nvme` driver

Fan paths are discovered at `/sys/devices/pci*/*/*/APP0001:00/fan*_input`

### Fan Control

The daemon:
1. Discovers all fans and reads min/max RPM values
2. Switches fans to manual control mode
3. Continuously polls sensor temperatures
4. Calculates target fan speeds based on configured curves
5. Applies smooth ramp-down to prevent oscillation

### Speed Curves

- **Linear**: Speed increases proportionally with temperature
- **Exponential**: Aggressive cooling at high temps (cubic curve)
- **Logarithmic**: Gentle initial response, then steeper

### Degraded Mode

If expected drivers are missing:
- Fans run at `initial_percent` speed
- After `escalation_delay` seconds, increases to `escalated_percent`
- Automatically recovers when drivers appear

## Sensor Naming

Sensors are named by driver and label:

| Driver | Example Names |
|--------|---------------|
| coretemp | "coretemp CPU0", "coretemp CPU1" |
| amdgpu | "amdgpu GPU", "amdgpu [0] GPU" (multi-GPU) |
| applesmc | "applesmc TC0P", "applesmc TG0D" |
| nvme | "nvme Composite", "nvme Sensor 1" |

## Safety Features

- **Root-only execution**: Prevents unauthorized hardware access
- **PID file locking**: Prevents multiple daemon instances
- **Graceful shutdown**: Returns fans to automatic mode on exit
- **Signal handling**: Responds to SIGINT/SIGTERM for clean termination
- **Degraded fallback**: Ensures minimum cooling even with sensor failures

## Troubleshooting

### No sensors found

```bash
# Check if hwmon is available
ls /sys/class/hwmon/

# Verify drivers are loaded
lspci -k | grep -A 3 -i vga
```

### Fan control fails

Ensure the fan paths exist:

```bash
ls /sys/devices/pci*/*/*/APP0001:00/
```

### Daemon won't start

Check for existing instance:

```bash
sudo pkill macproT2fans
sudo rm -f /run/macprot2fans.pid
```

## Building from Source

```bash
cargo build --release
```

For Nix users:

```bash
nix build
./result/bin/macproT2fans --list-sensors
```

## License

This project is provided as-is for use on Mac Pro 2019 systems. Use at your own risk.
