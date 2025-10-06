# ---- Build stage ----
FROM oven/bun:1.1 AS build
WORKDIR /app

# Install deps (better layer caching with lockfile)
COPY . ./
RUN bun install

# If your build script is "vite build", keep it; otherwise adjust:
RUN bun run build

# Expose the Vite preview port (default 4173)
EXPOSE 4173

# Use Vite's production preview server
# If your script is "preview": "vite preview --host --strictPort"
CMD ["bun", "run", "preview", "--host", "--strictPort"]