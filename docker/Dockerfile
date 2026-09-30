# SPDX-License-Identifier: MIT
FROM scratch
ARG TARGETARCH
ARG TARGETVARIANT
LABEL org.opencontainers.image.source="https://github.com/rwv/caj2pdf-rust" \
      org.opencontainers.image.description="Bounded-I/O CAJ-family to PDF command-line converter" \
      org.opencontainers.image.licenses="MIT"
COPY --chmod=0755 artifacts/${TARGETARCH}${TARGETVARIANT}/caj2pdf /usr/local/bin/caj2pdf
COPY artifacts/${TARGETARCH}${TARGETVARIANT}/LICENSE /usr/share/licenses/caj2pdf/LICENSE
COPY artifacts/${TARGETARCH}${TARGETVARIANT}/THIRD-PARTY-NOTICES.txt /usr/share/licenses/caj2pdf/THIRD-PARTY-NOTICES.txt
COPY --chmod=1777 rootfs/ /
WORKDIR /data
USER 65532:65532
ENV TMPDIR=/tmp
ENTRYPOINT ["/usr/local/bin/caj2pdf"]
CMD ["--help"]
