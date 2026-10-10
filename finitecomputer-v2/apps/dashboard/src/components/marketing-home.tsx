import Image from "next/image";

import styles from "./marketing-home.module.css";

export function MarketingHome() {
  return (
    <main className={styles.scene}>
      <div
        className={styles.background}
        role="img"
        aria-label="Wildflowers in a mountain meadow under bright clouds and blue sky"
      />

      <section className={styles.card} aria-label="Finite Computer">
        <div className={styles.logo} aria-label="Finite Computer logo">
          <Image
            src="/finite-logo.svg"
            alt=""
            className={styles.logoMark}
            width={72}
            height={72}
            aria-hidden="true"
            priority
          />
        </div>

        <div className={styles.cardBody}>
          <h1 className={styles.headline}>
            Finite makes frontier AI accessible to non-developers. We run in-person training and craft beautifully
            simple software to help humans be more human.
          </h1>
        </div>

        <div className={styles.actions}>
          <a href="/signup?returnTo=/dashboard" className={styles.button}>
            Sign up
          </a>
          <a href="/login?returnTo=/dashboard" className={styles.secondaryButton}>
            Sign in
          </a>
        </div>
      </section>
    </main>
  );
}
