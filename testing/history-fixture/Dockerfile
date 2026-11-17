# Container for the history acceptance scripts (testing/test-history-*.py).
#
# Those scripts assert the layout of the container they were written in:
# uid 65532, HOME=/work/home, private directories under /work, the Omarchy
# shell sources at /opt/history-omarchy, and debug binaries under
# /work/target/debug. This image reproduces that layout. Use run.sh next to
# this file to build it, build the binaries and run the scripts.
#
# Debian testing because the scripts need its Qt 6.11, GTK 4.24, Sway 1.12
# and Quickshell 0.3.
FROM debian:testing

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    # Build dependencies for voxtype and voxtype-meetings-gtk4
    build-essential ca-certificates clang cmake curl git libasound2-dev \
    libclang-dev libgtk-4-dev libssl-dev pkg-config \
    # Private display, session bus and accessibility for the GTK journeys
    adwaita-icon-theme at-spi2-core dbus-daemon dbus-x11 gir1.2-atspi-2.0 \
    gir1.2-gtk-4.0 x11-utils xauth xdotool xvfb \
    # Headless compositor and Quickshell for the QML journeys
    grim qt6-svg-plugins qt6-wayland quickshell seatd sway swaybg wayland-utils \
    qml6-module-qtqml qml6-module-qtqml-models qml6-module-qtqml-workerscript \
    qml6-module-qtquick qml6-module-qtquick-controls qml6-module-qtquick-layouts \
    qml6-module-qtquick-shapes qml6-module-qtquick-templates \
    qml6-module-qtquick-window qml6-module-qttest \
    # What the Python scripts import, and the font they render terminals with
    fonts-dejavu-core fonts-dejavu-mono procps python3 python3-gi python3-pil \
    python3-pyatspi python3-pyte \
    python3-pyside6.qtcore python3-pyside6.qtgui python3-pyside6.qtnetwork \
    python3-pyside6.qtopengl python3-pyside6.qtqml python3-pyside6.qtquick \
    python3-pyside6.qttest python3-pyside6.qtwidgets \
 && rm -rf /var/lib/apt/lists/*

# The same toolchain channel CI uses (dtolnay/rust-toolchain@stable).
ENV RUSTUP_HOME=/opt/rustup PATH=/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | CARGO_HOME=/opt/cargo sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path

# Omarchy's shell sources. test-history-omarchy-launch.py and the settings
# scripts load the real qs.Commons and qs.Ui types from this path.
ARG OMARCHY_COMMIT=74de10479d3bf7aa7865408e33219064cfe45432
RUN mkdir /opt/history-omarchy \
 && curl -fsSL https://github.com/omacom/omarchy/archive/${OMARCHY_COMMIT}.tar.gz \
  | tar -xz -C /opt/history-omarchy --strip-components=2 omarchy-${OMARCHY_COMMIT}/shell \
 && test -f /opt/history-omarchy/Commons/Style.qml \
 && chmod -R a-w /opt/history-omarchy

# The unprivileged test user and its private directories. run.sh mounts
# named volumes over /work/target and /work/cargo (they take this ownership
# when first created) and a host directory over /work/out.
RUN groupadd -g 65532 history && useradd -u 65532 -g 65532 -M -d /work/home history \
 && install -d -o 65532 -g 65532 -m 700 /work/home /work/runtime /work/scratch \
 && install -d -o 65532 -g 65532 /work/out /work/target /work/cargo

USER 65532:65532
WORKDIR /src
