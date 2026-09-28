# SPDX-License-Identifier: MIT
# Development-only image. Supply the opaque, verified vendor tree externally.
# Do not publish the image or place vendor bytes in the repository. Use only a
# dedicated external build context.
FROM --platform=linux/amd64 debian@sha256:f3034a6ec3c1205360777c4aae76234998866ad18806ae62b63a3f84ccad782b

RUN set -eu; \
    rm -f /etc/apt/sources.list.d/debian.sources; \
    printf '%s\n' 'deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/20250901T000000Z/ bookworm main' > /etc/apt/sources.list; \
    apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=20 update; \
    apt-get install --no-install-recommends -y \
      python3=3.11.2-1+b1 xvfb=2:21.1.7-3+deb12u9 \
      xdotool=1:3.20160805.1-5 xclip=0.13-2 x11-utils=7.7+5 \
      openbox=3.6.1-10 fontconfig=2.14.1-4 fonts-dejavu-core=2.37-6 \
      libxml2=2.9.14+dfsg-1.3~deb12u1 libxslt1.1=1.1.35-1+deb12u1 \
      hicolor-icon-theme=0.17-2 libnss3=2:3.87.1-1+deb12u1 \
      libxss1=1:1.2.3-1 libasound2=1.2.8-1+b1 libglib2.0-0=2.74.6-2+deb12u6 \
      libsm6=2:1.2.3-1 libice6=2:1.0.10-1 libxkbcommon-x11-0=1.5.0-1 \
      libxcb-cursor0=0.1.4-1 libxcb-xinerama0=1.15-1 \
      libxcb-render-util0=0.3.9-1+b1 libxcb-image0=0.4.0-2 \
      libxcb-keysyms1=0.4.0-1+b2 libxcb-icccm4=0.4.1-1.1 \
      libxcb-shape0=1.15-1 libxcb-xfixes0=1.15-1 libgl1=1.6.0-1 libegl1=1.6.0-1; \
    rm -rf /var/lib/apt/lists/*; \
    mkdir -p /home/canary /input /output /runtime; \
    printf '%s\n' 'canary:x:1000:1000:canary:/home/canary:/bin/sh' >> /etc/passwd; \
    printf '%s\n' 'canary:x:1000:' >> /etc/group; \
    chown 1000:1000 /home/canary /input /output /runtime

# Vendor documentation/sample files and all installation hooks are omitted.
COPY vendor/opt/cajviewer/ /opt/cajviewer/
COPY cajviewer_canary.py /opt/canary/cajviewer_canary.py
COPY cajviewer_session.py /opt/canary/cajviewer_session.py
USER 1000:1000
WORKDIR /home/canary
ENV HOME=/home/canary DISPLAY=:99 LANG=C.UTF-8 LC_ALL=C.UTF-8 TZ=UTC \
    XDG_RUNTIME_DIR=/runtime XDG_CONFIG_HOME=/home/canary/.config \
    XDG_CACHE_HOME=/home/canary/.cache
ENTRYPOINT ["python3", "/opt/canary/cajviewer_session.py"]
