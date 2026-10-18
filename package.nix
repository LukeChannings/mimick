{
  lib,
  craneLib,
  rustPlatform,
  pkg-config,
  cmake,
  nasm,
  wrapGAppsHook4,
  glib,
  gtk4,
  libadwaita,
  openssl,
  libheif,
  webp-pixbuf-loader,
  gdk-pixbuf,
  librsvg,
  libwebp,
  openjpeg,
  libjpeg_turbo,
  dbus,
  adwaita-icon-theme,
  shared-mime-info,
}:

let
  fs = lib.fileset;

  # Everything the compile actually reads, so docs/wiki/nix edits never
  # invalidate the source hash.
  src = fs.toSource {
    root = ./.;
    fileset = fs.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./build.rs
      ./clippy.toml
      ./src
      ./setup
      ./tests
      ./THIRD_PARTY_LICENSES_SUMMARY.txt
    ];
  };

  # Shared by the deps-only and final builds: crane requires both to see the
  # same toolchain and C libraries, or the artifact cache misses.
  commonArgs = {
    pname = "mimick";
    inherit ((lib.importTOML ./Cargo.toml).package) version;
    inherit src;
    strictDeps = true;

    nativeBuildInputs = [
      pkg-config
      cmake
      nasm
      rustPlatform.bindgenHook
      wrapGAppsHook4
      glib # glib-compile-resources, used by build.rs
    ];

    buildInputs = [
      glib
      gtk4
      libadwaita
      openssl
      libheif
      libwebp
      openjpeg
      libjpeg_turbo
      dbus
      adwaita-icon-theme
      shared-mime-info
    ];

    # cmake is only needed by the -sys crates' build scripts, not for the
    # top-level build.
    dontUseCmakeConfigure = true;

    # Tests need a display / D-Bus session and read fixtures at runtime.
    doCheck = false;
  };

  # All dependency crates, compiled against a stubbed-out crate (crane dummies
  # src/ and build.rs). Only Cargo.toml/Cargo.lock changes rebuild this, so a
  # source edit reuses the compiled deps and recompiles Mimick alone.
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    # Immich serves WebP thumbnails, which gdk-pixbuf can only decode with the
    # webp-pixbuf-loader module. The loaders.cache that wrapGAppsHook picks up
    # (librsvg's) lists only the stock loaders plus SVG, so build a cache that
    # also includes WebP and point the wrapper at it.
    postInstall = ''
      install -d "$out/share/mimick"
      ${lib.getDev gdk-pixbuf}/bin/gdk-pixbuf-query-loaders \
        ${gdk-pixbuf}/${gdk-pixbuf.moduleDir}/*.so \
        ${lib.getLib librsvg}/${gdk-pixbuf.moduleDir}/*.so \
        ${webp-pixbuf-loader}/${gdk-pixbuf.moduleDir}/*.so \
        > "$out/share/mimick/loaders.cache"
      # gappsWrapperArgsHook (a preFixupPhase, after installPhase) reads this.
      export GDK_PIXBUF_MODULE_FILE="$out/share/mimick/loaders.cache"

      install -Dm644 setup/dev.nicx.mimick.desktop \
        "$out/share/applications/dev.nicx.mimick.desktop"
      install -Dm644 setup/metainfo/dev.nicx.mimick.metainfo.xml \
        "$out/share/metainfo/dev.nicx.mimick.metainfo.xml"
      install -Dm644 src/assets/icon.svg \
        "$out/share/icons/hicolor/scalable/apps/dev.nicx.mimick.svg"
      install -Dm644 src/assets/256x256/apps/dev.nicx.mimick.png \
        "$out/share/icons/hicolor/256x256/apps/dev.nicx.mimick.png"
      install -Dm644 src/assets/512x512/apps/dev.nicx.mimick.png \
        "$out/share/icons/hicolor/512x512/apps/dev.nicx.mimick.png"
    '';

    meta = {
      description = "Unofficial Immich desktop client and auto-sync agent";
      homepage = "https://github.com/nicx17/mimick";
      license = lib.licenses.gpl3Plus;
      platforms = lib.platforms.linux;
      mainProgram = "mimick";
    };
  }
)
