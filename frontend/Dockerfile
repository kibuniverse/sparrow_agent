# ============================================
# Stage 1: Build the frontend
# ============================================
FROM node:22-alpine AS builder

# Install pnpm globally
RUN corepack enable && corepack prepare pnpm@latest --activate

WORKDIR /app

# Copy dependency manifests first (better layer caching)
COPY package.json pnpm-lock.yaml ./

# Install dependencies
RUN pnpm install --frozen-lockfile

# Copy the rest of the source code
COPY . .

# Build the project
RUN pnpm run build

# ============================================
# Stage 2: Serve with Nginx
# ============================================
FROM nginx:stable-alpine

# Remove default nginx static assets
RUN rm -rf /usr/share/nginx/html/*

# Copy built files from the builder stage
COPY --from=builder /app/dist /usr/share/nginx/html

# Copy custom nginx configuration
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Expose port 80
EXPOSE 80

# Start nginx in foreground
CMD ["nginx", "-g", "daemon off;"]
