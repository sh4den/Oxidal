Name:           oxidal
Version:        %{version}
Release:        1
Summary:        Cross-platform SSH, SFTP and serial terminal client
License:        GPL-3.0-only
URL:            https://github.com/sh4den/Oxidal

%global debug_package %{nil}
%global __strip /bin/true

Requires:       vulkan-loader
Requires:       fontconfig
Requires:       hicolor-icon-theme
Recommends:     libwayland-client
Recommends:     gnome-keyring

%description
Oxidal puts SSH, SFTP, telnet, serial and local shells side by side in a
single native window.

%install
install -Dm755 %{_sourcedir}/oxidal %{buildroot}%{_bindir}/oxidal
install -Dm644 %{_sourcedir}/oxidal.desktop %{buildroot}%{_datadir}/applications/oxidal.desktop
install -dm755 %{buildroot}%{_datadir}/metainfo
sed 's/io\.github\.sh4den\.Oxidal\.desktop/oxidal.desktop/' \
  %{_sourcedir}/io.github.sh4den.Oxidal.metainfo.xml > %{buildroot}%{_datadir}/metainfo/oxidal.metainfo.xml
for size in 16 32 48 64 128 256 512; do
  install -Dm644 %{_sourcedir}/icons/${size}.png \
    %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/oxidal.png
done

%files
%{_bindir}/oxidal
%{_datadir}/applications/oxidal.desktop
%{_datadir}/metainfo/oxidal.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/oxidal.png
