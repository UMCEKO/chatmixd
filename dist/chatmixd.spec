%global commit      82f42e5c63afef2acfa37a29ab8972c98ca708d4
%global shortcommit %(c=%{commit}; echo ${c:0:7})
%global snapdate    20260601

Name:           chatmixd
Version:        0.2.0
Release:        3.%{snapdate}git%{shortcommit}%{?dist}
Summary:        SteelSeries ChatMix daemon for Linux (PipeWire/PulseAudio)

License:        MIT
URL:            https://github.com/UMCEKO/chatmixd
Source0:        %{url}/archive/%{commit}/%{name}-%{shortcommit}.tar.gz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  systemd-rpm-macros

# pactl + pw-cli are invoked at runtime.
Requires:       pipewire-pulseaudio
Requires:       pulseaudio-utils

%description
chatmixd splits a SteelSeries headset's ChatMix dial into separate Game and
Chat virtual sinks under PipeWire, so the hardware dial mixes desktop audio
against voice/chat the same way it does on Windows. Hardened fork of linuxmix.

%prep
%autosetup -n %{name}-%{commit}

%build
# The crate has no third-party dependencies, so the build needs no network.
cargo build --release --offline

%install
install -Dm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm0644 dist/%{name}.service    %{buildroot}%{_userunitdir}/%{name}.service
install -Dm0644 dist/99-%{name}.rules   %{buildroot}%{_udevrulesdir}/99-%{name}.rules

%files
%license LICENSE
%doc README.md
%{_bindir}/%{name}
%{_userunitdir}/%{name}.service
%{_udevrulesdir}/99-%{name}.rules

%changelog
* Mon Jun 01 2026 UMCEKO <umutcevdetkocak@gmail.com> - 0.2.0-3.20260601git82f42e5
- Snapshot 82f42e5: log actionable errors instead of silent None

* Mon Jun 01 2026 UMCEKO <umutcevdetkocak@gmail.com> - 0.2.0-2.20260601gitbeb0851
- Snapshot beb0851: uaccess udev rule (no audio-group membership needed)

* Mon Jun 01 2026 UMCEKO <umutcevdetkocak@gmail.com> - 0.2.0-1.20260601gitf55d034
- Initial RPM packaging (git snapshot)
