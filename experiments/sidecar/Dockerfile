FROM golang:1.24-alpine AS builder
WORKDIR /src
COPY go.mod main.go ./
RUN CGO_ENABLED=0 GOOS=linux go build -ldflags="-s -w" -o /readiness-sidecar .

FROM scratch
COPY --from=builder /readiness-sidecar /readiness-sidecar
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/readiness-sidecar"]
