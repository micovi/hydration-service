# Use the official Bun image
FROM oven/bun:latest

# Set working dir
WORKDIR /app

# Copy package files first to leverage Docker cache for dependencies
COPY . ./

# Install dependencies with Bun
RUN bun install

# Copy the rest of the project
COPY . .

# Expose Vite default port
EXPOSE 5173

# Run Vite dev server and bind to all interfaces so container is reachable
CMD ["bun", "run", "dev", "--", "--host", "0.0.0.0", "--port", "5173"]