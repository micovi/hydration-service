# ---- Build stage ----
FROM oven/bun:latest AS build
WORKDIR /app

# Install deps (better layer caching with lockfile)
COPY . ./
RUN bun install

# If your build script is "vite build", keep it; otherwise adjust:
RUN bunx --bun vite build

# Expose the Vite preview port (default 4173)
EXPOSE 4173

# Use Vite's production preview server
# If your script is "preview": "vite preview --host --strictPort"
CMD ["bun", "run", "preview", "--host", "--strictPort"]