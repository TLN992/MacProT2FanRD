{
  description = "NixOS module and package for macproT2fans";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "macproT2fans";
        version = "0.1.0";
        src = ./.;
        # You may need to update this hash after your first build attempt
        cargoHash = "sha256-r63OdZ7aoFagDKM3RvVCUndlWM+/g8c9vpwPt9SgZTA=";
      };

      nixosModules.macprot2fans = { config, lib, pkgs, ... }:
        let
          cfg = config.services.macprot2fans;
        in {
          options.services.macprot2fans = {
            enable = lib.mkEnableOption "macproT2fans daemon to manage fan curves for T2 Macs";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${system}.default;
              description = "macproT2fans package to use.";
            };

            defaults = lib.mkOption {
              type = lib.types.submodule {
                options = {
                  low_temp = lib.mkOption { type = lib.types.int; default = 55; };
                  high_temp = lib.mkOption { type = lib.types.int; default = 75; };
                  speed_curve = lib.mkOption { type = lib.types.enum [ "linear" "exponential" "logarithmic" ]; default = "linear"; };
                  always_full_speed = lib.mkOption { type = lib.types.bool; default = false; };
                  sensor_aggregation = lib.mkOption { type = lib.types.enum [ "max" "average" ]; default = "max"; };
                  ramp_down_rate = lib.mkOption { type = lib.types.float; default = 1.0; };
                };
              };
              default = {};
            };

            fans = lib.mkOption {
              type = lib.types.attrsOf (lib.types.submodule {
                options = {
                  low_temp = lib.mkOption { type = lib.types.nullOr lib.types.int; default = null; };
                  high_temp = lib.mkOption { type = lib.types.nullOr lib.types.int; default = null; };
                  speed_curve = lib.mkOption { type = lib.types.nullOr (lib.types.enum [ "linear" "exponential" "logarithmic" ]); default = null; };
                  sensor_aggregation = lib.mkOption { type = lib.types.nullOr (lib.types.enum [ "max" "average" ]); default = null; };
                  ramp_down_rate = lib.mkOption { type = lib.types.nullOr lib.types.float; default = null; };
                  sensors = lib.mkOption { type = lib.types.nullOr (lib.types.listOf lib.types.str); default = null; };
                };
              });
              default = {};
            };

            degraded = lib.mkOption {
              type = lib.types.submodule {
                options = {
                  expected_drivers = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ "coretemp" "amdgpu" ]; };
                  initial_percent = lib.mkOption { type = lib.types.int; default = 60; };
                  escalated_percent = lib.mkOption { type = lib.types.int; default = 80; };
                  escalation_delay = lib.mkOption { type = lib.types.int; default = 60; };
                };
              };
              default = {};
            };
          };

          config = lib.mkIf cfg.enable {
            systemd.services.macprot2fans = {
              description = "macproT2fans daemon to manage the fans on a T2 Mac";
              wantedBy = [ "multi-user.target" ];
              serviceConfig = {
                Type = "exec";
                ExecStart = "${cfg.package}/bin/macproT2fans";
                Restart = "always";

                # Security hardening from Fedora service example
                PrivateTmp = "true";
                ProtectSystem = "true";
                ProtectHome = "true";
                ProtectClock = "true";
                ProtectControlGroups = "true";
                ProtectHostname = "true";
                ProtectKernelLogs = "true";
                ProtectKernelModules = "true";
                ProtectProc = "invisible";
                PrivateDevices = "true";
                PrivateNetwork = "true";
                NoNewPrivileges = "true";
                DevicePolicy = "closed";
                KeyringMode = "private";
                LockPersonality = "true";
                MemoryDenyWriteExecute = "true";
                PrivateUsers = "yes";
                RemoveIPC = "yes";
                RestrictNamespaces = "yes";
                RestrictRealtime = "yes";
                RestrictSUIDSGID = "yes";
                SystemCallArchitectures = "native";
              };
            };

            environment.etc."macprot2fans.toml".text = (pkgs.formats.toml { }).generate "macprot2fans.toml" {
              defaults = cfg.defaults;
              fan = cfg.fans;
              degraded = cfg.degraded;
            };
          };
        };
    };
}
