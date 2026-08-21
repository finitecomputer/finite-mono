# finite-search — SearXNG + Firecrawl, mirroring the lat2 compose projects
# (finite-search/compose/, capture host-capture/lat2/search-compose.txt).
# Loopback-only, same ports as lat2: SearXNG 127.0.0.1:8080, Firecrawl API
# 127.0.0.1:3002. FoundationDB is omitted (experimental NUQ_BACKEND=fdb path
# was not active; the queue runs on nuq-postgres as captured).
#
# Images below are pinned to the linux/amd64 manifest digests recorded from
# finite-lat-1's podman RepoDigests on 2026-08-14.
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
      # Source tag docker.io/searxng/searxng:latest; digest recorded from lat1 on 2026-08-14.
      image = "docker.io/searxng/searxng@sha256:70e5d035b085b1a2da116d145e1cba4425cadd317b072daa4385e8d7f7e21062";
      ports = [ "127.0.0.1:8080:8080" ];
      volumes = [ "${searxngSettings}:/etc/searxng/settings.yml:ro" ];
      # NAMES only (values from lat2 /home/ubuntu/finite-search/searxng/.env):
      #   SEARXNG_SECRET
      #   SEARXNG_BASE_URL   (optional)
      #   SEARXNG_LIMITER    (optional, false)
      environmentFiles = [ "/etc/finite/searxng.env" ];
    };

    firecrawl-redis = {
      # Source tag docker.io/library/redis:alpine; digest recorded from lat1 on 2026-08-14.
      image = "docker.io/library/redis@sha256:cd5f3ac681c77791c6a8eaa62de876ad2be043ee5a428afb7c0095aa08246277";
      cmd = [
        "redis-server"
        "--bind"
        "0.0.0.0"
      ];
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-rabbitmq = {
      # Source tag docker.io/library/rabbitmq:3-management; digest recorded from lat1 on 2026-08-14.
      image = "docker.io/library/rabbitmq@sha256:9cfb7e92ae7d296aec4d1ae799e431209f7ed57d55f9c929d95667d0ccf1c920";
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-nuq-postgres = {
      # Source tag ghcr.io/firecrawl/nuq-postgres:latest; digest recorded from lat1 on 2026-08-14.
      image = "ghcr.io/firecrawl/nuq-postgres@sha256:4ca6718b2cef40404b046db5cd37ae45db3e44d1a5750c80522f3587a5b193d5";
      volumes = [ "firecrawl-nuq-postgres-data:/var/lib/postgresql/data" ];
      # NAMES only (values from lat2 firecrawl-upstream/.env):
      #   POSTGRES_USER / POSTGRES_PASSWORD / POSTGRES_DB
      environmentFiles = [ "/etc/finite/firecrawl.env" ];
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-playwright = {
      # Source tag ghcr.io/firecrawl/playwright-service:latest; digest recorded from lat1 on 2026-08-14.
      image = "ghcr.io/firecrawl/playwright-service@sha256:c13a0e147e8b6a503093d68edfb223ac65c989058f7e0ef606ee2958b38ff604";
      environment = {
        PORT = "3000";
        MAX_CONCURRENT_PAGES = "10";
      };
      extraOptions = [ "--network=firecrawl" ];
    };

    firecrawl-api = {
      # Source tag ghcr.io/firecrawl/firecrawl:latest; digest recorded from lat1 on 2026-08-14.
      image = "ghcr.io/firecrawl/firecrawl@sha256:e7c96367e8e6f783405f52c24d1c44daac06415679b3be724fe46c0730fc0504";
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
