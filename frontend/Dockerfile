# Use the official Bun image for building
FROM oven/bun:latest as builder

# Set working dir
WORKDIR /app

# Copy package files first to leverage Docker cache for dependencies
COPY package.json bun.lockb ./

# Install dependencies with Bun
RUN bun install

# Copy the rest of the project
COPY . .

# Build the project for production
RUN bun run build

# Use nginx to serve the built files
FROM nginx:alpine

# Copy the built files from the builder stage
COPY --from=builder /app/dist /usr/share/nginx/html

# Copy custom nginx config if needed (optional)
# COPY nginx.conf /etc/nginx/nginx.conf

# Expose port 8080
EXPOSE 8080

# Create custom nginx config to run on port 8080
RUN echo 'server { listen 8080; location / { root /usr/share/nginx/html; index index.html; try_files $uri $uri/ /index.html; } }' > /etc/nginx/conf.d/default.conf

# Start nginx
CMD ["nginx", "-g", "daemon off;"]