import logging

from . import adapter


def register(ctx):
    adapter.register(ctx)
    try:
        from . import simplex_topics

        simplex_topics.register(ctx)
    except Exception:
        # Optional SimpleX integration must never prevent Finite Chat loading.
        # The stock managed runtime keeps group traffic disabled on this path.
        logging.getLogger(__name__).exception("Owner-only SimpleX topics are unavailable")


__all__ = ["register"]
