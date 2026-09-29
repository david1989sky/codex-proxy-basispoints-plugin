FROM mcr.microsoft.com/playwright:v1.55.0-noble
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --omit=dev --ignore-scripts --no-audit --no-fund
COPY src ./src
COPY entrypoint.sh ./entrypoint.sh
RUN chmod 0555 ./entrypoint.sh
USER pwuser
ENV NODE_ENV=production HOME=/tmp
CMD ["./entrypoint.sh"]
