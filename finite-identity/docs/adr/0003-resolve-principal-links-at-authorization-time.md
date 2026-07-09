# Resolve Principal Links at authorization time

When an Email-Only Principal later links to a Native Principal, products are not required to immediately rewrite existing product-owned permission records. Finite Identity exposes Principal Resolution so products can authorize a native caller against email-shaped grants at access time, while products may still compact or rewrite their own records later as an optimization.
