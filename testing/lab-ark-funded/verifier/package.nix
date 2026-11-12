# Frozen offline workspace-compatible lock. Compile only in the assigned slot.
{ pkgs, rust }:
assert builtins.hashFile "sha256" ./Cargo.lock == "ac24ee3b09c7c2dd8731417672b9d7b072aa6e9e04127b9c96e8ad80ae842833";
pkgs.rustPlatform.buildRustPackage {
  pname = "ap5-vtxo-verifier";
  version = "0.1.0";
  src = pkgs.lib.cleanSourceWith {
    src = ./.;
    filter = path: type: !(builtins.elem (baseNameOf path) [ "target" ".git" ]);
  };
  cargo = rust;
  rustc = rust;
  cargoLock = { lockFile = ./Cargo.lock; allowBuiltinFetchGit = true; };
  cargoBuildFlags = [ "--bin" "ap5-vtxo-verifier" ];
  doCheck = true;
  postInstall = ''
    mkdir -p $out/share/ap5-verifier
    cp src/main.rs src/vectors.rs Cargo.lock Cargo.toml $out/share/ap5-verifier/
    cp -r fixtures $out/share/ap5-verifier/
  '';
  meta.mainProgram = "ap5-vtxo-verifier";
}
