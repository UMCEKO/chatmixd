{
  lib,
  rustPlatform,
  makeWrapper,
  pulseaudio,
  pipewire,
}:

rustPlatform.buildRustPackage {
  pname = "chatmixd";
  version = "0.2.0";

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../dist
      ../LICENSE
      ../README.md
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];

  postInstall = ''
    install -Dm0644 dist/99-chatmixd.rules $out/lib/udev/rules.d/99-chatmixd.rules
    install -Dm0644 dist/chatmixd.service $out/lib/systemd/user/chatmixd.service
    substituteInPlace $out/lib/systemd/user/chatmixd.service \
      --replace-fail /usr/bin/chatmixd $out/bin/chatmixd

    # pactl and pw-cli are invoked at runtime via $PATH.
    wrapProgram $out/bin/chatmixd \
      --prefix PATH : ${lib.makeBinPath [ pulseaudio pipewire ]}
  '';

  meta = {
    description = "SteelSeries ChatMix daemon for Linux (PipeWire/PulseAudio)";
    homepage = "https://github.com/UMCEKO/chatmixd";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "chatmixd";
  };
}
