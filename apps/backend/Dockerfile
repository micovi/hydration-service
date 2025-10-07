# ---- Build stage ----
FROM oven/bun:latest AS build
WORKDIR /app

# Install deps (better layer caching with lockfile)
COPY . ./
RUN bun install

EXPOSE 8081

# Use Vite's production preview server
# If your script is "preview": "vite preview --host --strictPort"
CMD ["bun", "run", "src/index.ts"]