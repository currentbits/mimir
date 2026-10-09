# Self-hosted mimir: all your bots 24/7 + web UI. See README "Self-hosting".
# Base images pinned by digest (the tag says which; update both together).
FROM node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392 AS ui
WORKDIR /ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY ui/ ./
RUN npm run build

FROM rust:1-bookworm@sha256:114c7a4425406451c2866b6aafe69fe29b1b298832db1277d411ac73c82d04d6 AS bin
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Only the CLI/server; the desktop crate is never compiled here.
RUN cargo build --release -p mimir

# Node runtime so the official AI CLIs (claude, codex, grok) can run here with your own login.
FROM node:22-bookworm-slim@sha256:c3de60bf2f9dd0ac6370e6117950ff62d6e339527e7472301c9c78a017978392
# Fixed versions: what runs your bots, and holds their logins, doesn't change on a rebuild. Bump deliberately.
ARG AI_CLIS="@anthropic-ai/claude-code@2.1.293 @openai/codex@0.161.0"
# Grok Build (SuperGrok / X Premium+), only when asked for: --build-arg GROK_CLI=1. Its installer is a script from x.ai
# run as root during the build, with no fixed version or checksum, so it isn't in the image by default. Installed
# outside /data so the volume doesn't hide it; Mimir keeps its grok login in /data/grok.
ARG GROK_CLI=
# The bots' own computer: a virtual screen (Xvfb) with a window manager and a visible Chromium they can see and use,
# and you can watch in the web app (Settings → Computer). Build with --build-arg COMPUTER= to leave it out.
# docker-compose.yml runs it as a service of its own (`mimir computer`, user computer, no /data); a plain
# `docker run` keeps it inside Mimir's container.
ARG COMPUTER=1
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl tini \
 && if [ -n "$COMPUTER" ]; then apt-get install -y --no-install-recommends xvfb xdotool scrot openbox chromium fonts-dejavu-core fonts-noto-color-emoji fonts-noto-cjk \
      && mkdir -p /etc/chromium/policies/managed \
      && echo '{"URLBlocklist":["file://*","chrome://*","view-source:*","devtools://*"],"URLAllowlist":["chrome://newtab","chrome://new-tab-page","chrome://new-tab-page-third-party"],"DownloadRestrictions":3}' > /etc/chromium/policies/managed/mimir.json; fi \
 && rm -rf /var/lib/apt/lists/* \
 && if [ -n "$COMPUTER" ]; then npm install -g --no-audit --no-fund @playwright/mcp@0.0.83 && npm cache clean --force; fi \
 && if [ -n "$AI_CLIS" ]; then npm install -g --no-audit --no-fund $AI_CLIS && npm cache clean --force; fi \
 && if [ -n "$GROK_CLI" ]; then curl -fsSL https://x.ai/cli/install.sh | HOME=/opt/grok GROK_BIN_DIR=/usr/local/bin bash && chmod -R a+rX /opt/grok; fi \
 && useradd --create-home --home-dir /data/home --uid 10001 mimir && mkdir -p /data && chown -R mimir /data \
 && useradd --create-home --home-dir /home/computer --uid 10002 computer
COPY --from=bin /src/target/release/mimir /usr/local/bin/mimir
COPY --from=ui /ui/dist /app/ui
USER mimir
# Database (memories, bots, tokens) in /data; CLI logins (~/.claude, ~/.codex) in /data/home, Mimir's grok login in /data/grok. Mount /data as a volume.
ENV MIMIR_HOME=/data HOME=/data/home MIMIR_UI_DIR=/app/ui
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s CMD node -e "fetch('http://127.0.0.1:8080/healthz').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"
# tini reaps the computer's processes (the screen, Chromium) if they exit, since mimir would otherwise be PID 1.
ENTRYPOINT ["/usr/bin/tini", "--"]
CMD ["mimir", "server", "--listen", "0.0.0.0:8080"]
