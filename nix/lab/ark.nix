{ pkgs, rust }:
let
  rev = "c419159fa54085572f8ca72d2ba1716556a9d3eb";
  # Stock upstream source. No runtime downloads and no Ark protocol patches.
  src = pkgs.fetchgit {
    url = "https://gitlab.com/ark-bitcoin/bark.git";
    inherit rev;
    hash = "sha256-qmdyctM8mZ27IKruiGcZ42OmvHwYm0enlPqVWnw7WkU=";
  };
  mk = pname: package: binary: pkgs.rustPlatform.buildRustPackage {
    inherit pname src;
    version = "0.7.1-${builtins.substring 0 7 rev}";
    rustc = rust;
    cargo = rust;
    # The complete upstream lock covers registry checksums and exact git SHAs.
    cargoLock = {
      lockFile = "${src}/Cargo.lock";
      allowBuiltinFetchGit = true;
    };
    nativeBuildInputs = [ rust pkgs.pkg-config pkgs.protobuf ];
    buildInputs = [ pkgs.openssl pkgs.sqlite pkgs.postgresql_17 ];
    PROTOC = "${pkgs.protobuf}/bin/protoc";
    GIT_HASH = rev;
    SERVER_VERSION = "0.7.1-dev";
    BARK_VERSION = "0.7.1-dev";
    cargoBuildFlags = [ "-p" package "--bin" binary ]
      # Only the CLI is needed here, not barkd's downloadable Swagger UI.
      ++ pkgs.lib.optionals (package == "bark-cli")
        [ "--no-default-features" "--features" "tls-native-roots" ];
    doCheck = false; # Upstream integration tests require a funded regtest chain.
    postInstall = ''
      mkdir -p $out/share/ark
      cp -r server-rpc/protos $out/share/ark/
      cp server/captaind.default.toml $out/share/ark/
      cp Cargo.lock $out/share/ark/Cargo.lock
      printf '%s\n' '${rev}' > $out/share/ark/source-revision
      $out/bin/${binary} --version > $out/share/ark/version
    '';
    passthru = { inherit rev; upstreamSource = src; };
    meta.mainProgram = binary;
  };
in {
  captaind = mk "captaind" "bark-server" "captaind";
  bark = mk "bark" "bark-cli" "bark";
}
