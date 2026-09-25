import logging

from . import adapter


def register(ctx):
    adapter.register(ctx)
    try:
        from hermes_cli.config import load_config_readonly

        # Managed SimpleX is optional. Looking it up in the lazy registry loads
        # its plugin; newer Hermes loaders do that on a worker that cannot take
        # the discovery lock held by the thread waiting for this registration.
        # Agents without managed SimpleX must never enter that loading path.
        config = load_config_readonly()
        simplex = config.get("gateway", {}).get("platforms", {}).get("simplex", {})
        if (
            simplex.get("enabled") is not True
            or simplex.get("extra", {}).get("finite_managed") is not True
        ):
            return
        from . import simplex_topics

        simplex_topics.register(ctx)
    except Exception:
        # Optional SimpleX integration must never prevent Finite Chat loading.
        # The stock managed runtime keeps group traffic disabled on this path.
        logging.getLogger(__name__).exception("Owner-only SimpleX topics are unavailable")


__all__ = ["register"]
