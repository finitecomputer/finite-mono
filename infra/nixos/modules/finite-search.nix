# finite-search — SearXNG + Firecrawl, mirroring the lat2 compose projects
# (finite-search/compose/, capture host-capture/lat2/search-compose.txt).
# Loopback-only, same ports as lat2: SearXNG 127.0.0.1:8080, Firecrawl API
# 127.0.0.1:3002. FoundationDB is omitted (experimental NUQ_BACKEND=fdb path
# was not active; the queue runs on nuq-postgres as captured).
#
# TODO: digest-pin every image below. lat2 ran searxng/searxng:latest and
# BUILT the firecrawl images on-box from the upstream checkout; here we take
# the upstream GHCR images by tag until CI mirrors/pins them.
{ pkgs, ... }:
let
  # Replicates lat2's /home/ubuntu/finite-search/searxng/settings.yml.
  searxngSettings = pkgs.writeText "searxng-settings.yml" ''
    use_default_settings:
      engines:
        keep_only:
          - bing
          - yep
          - mwmbl
          - searchmysite
          - wiby
          - presearch
          - github
          - mdn
          - stackoverflow
          - askubuntu
          - pubmed
          - reuters
          - bing news
          - docker hub
          - mankier
          - openlibrary
          - wikiquote
          - wikibooks
          - wikinews
          - wiktionary
          - searchch

    general:
      debug: false
      instance_name: "finite-search"

    search:
      safe_search: 0
      formats:
        - html
        - json

    server:
      bind_address: "0.0.0.0"
      port: 8080
      limiter: false
      public_instance: false
      secret_key: "''${SEARXNG_SECRET}"

    ui:
      static_use_hash: true
  '';
in
{
  virtualisation.oci-containers.containers = {
    searxng = {
      image = "searxng/searxng:latest"; # TODO: digest-pin
      ports = [ "127.0.0.1:8080:8080" ];
      volumes = [ "${searxngSettings}:/etc/searxng/settings.yml:ro" ];
      # NAMES only (values from lat2 /home/ubuntu/finite-search/searxng/.env):
      #   SEARXNG_SECRET
      #   SEARXNG_BASE_URL   (optional)
      #   SEARXNG_LIMITER    (optional, false)
      environmentFiles = [ "/etc/finite/searxng.env" ];
    };

    firecrawl-redis = {
      image = "docker.io/library/redis:alpine"; # TODO: digest-pin
      cmd = [
        "redis-server"
        "--bind"
        "0.0.0.0"
      ];
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-rabbitmq = {
      image = "docker.io/library/rabbitmq:3-management"; # TODO: digest-pin
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-nuq-postgres = {
      image = "ghcr.io/firecrawl/nuq-postgres:latest"; # TODO: digest-pin (lat2 built this on-box)
      volumes = [ "firecrawl-nuq-postgres-data:/var/lib/postgresql/data" ];
      # NAMES only (values from lat2 firecrawl-upstream/.env):
      #   POSTGRES_USER / POSTGRES_PASSWORD / POSTGRES_DB
      environmentFiles = [ "/etc/finite/firecrawl.env" ];
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-playwright = {
      image = "ghcr.io/firecrawl/playwright-service:latest"; # TODO: digest-pin (lat2 built this on-box)
      environment = {
        PORT = "3000";
        MAX_CONCURRENT_PAGES = "10";
      };
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-api = {
      image = "ghcr.io/firecrawl/firecrawl:latest"; # TODO: digest-pin (lat2 built this on-box)
      ports = [ "127.0.0.1:3002:3002" ];
      dependsOn = [
        "firecrawl-redis"
        "firecrawl-rabbitmq"
        "firecrawl-nuq-postgres"
        "firecrawl-playwright"
      ];
      environment = {
        HOST = "0.0.0.0";
        PORT = "3002";
        REDIS_URL = "redis://firecrawl-redis:6379";
        REDIS_RATE_LIMIT_URL = "redis://firecrawl-redis:6379";
        PLAYWRIGHT_MICROSERVICE_URL = "http://firecrawl-playwright:3000/scrape";
        NUQ_RABBITMQ_URL = "amqp://firecrawl-rabbitmq:5672";
        POSTGRES_HOST = "firecrawl-nuq-postgres";
        POSTGRES_PORT = "5432";
        USE_DB_AUTHENTICATION = "false";
        SEARXNG_ENDPOINT = "http://host.containers.internal:8080";
      };
      # NAMES only (values from lat2 firecrawl-upstream/.env):
      #   BULL_AUTH_KEY
      #   POSTGRES_USER / POSTGRES_PASSWORD / POSTGRES_DB
      #   MAX_CPU / MAX_RAM
      environmentFiles = [ "/etc/finite/firecrawl.env" ];
      extraOptions = [ "--network=firecrawl" ];
    };
  };

  # Named podman network so the firecrawl containers resolve each other by
  # container name (compose gave lat2 the same via its default network).
  systemd.services.init-firecrawl-network = {
    description = "Create the podman network for firecrawl";
    wantedBy = [ "multi-user.target" ];
    before = [
      "podman-firecrawl-redis.service"
      "podman-firecrawl-rabbitmq.service"
      "podman-firecrawl-nuq-postgres.service"
      "podman-firecrawl-playwright.service"
      "podman-firecrawl-api.service"
    ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
    };
    script = ''
      ${pkgs.podman}/bin/podman network exists firecrawl \
        || ${pkgs.podman}/bin/podman network create firecrawl
    '';
  };
}
