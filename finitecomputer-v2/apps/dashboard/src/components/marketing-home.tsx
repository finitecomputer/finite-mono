import Image from "next/image";

import styles from "./marketing-home.module.css";

const CONTACT_FORM_URL =
  "https://docs.google.com/forms/d/e/1FAIpQLSePGnux9EVHRGZf30q7MPEMdMmTb7djJxAPCM0hCf-wRTGv3w/viewform?usp=publish-editor";

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

        <a href={CONTACT_FORM_URL} className={styles.button} target="_blank" rel="noopener noreferrer">
          Get in touch
        </a>
      </section>
    </main>
  );
}
