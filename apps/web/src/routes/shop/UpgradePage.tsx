import { useState } from "react";
import { api } from "../../api/client";
import styles from "../pages.module.css";

/** Sells "Nothing to See Here". Stripe hosts the card form; we only get a redirect URL. */
export function UpgradePage() {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function buy() {
    setPending(true);
    setError(null);
    try {
      const { url } = await api.createCheckout();
      window.location.assign(url);
    } catch (err) {
      setError(String(err));
      setPending(false);
    }
  }

  return (
    <section className={styles.page}>
      <header>
        <h1>Nothing to See Here</h1>
        <p className={styles.intro}>
          Premium emptiness for your support inbox. The Backlog Beast stays asleep, and so do you.
        </p>
      </header>
      <div className={styles.card}>
        <p>¥100 per month. Payments are handled by Stripe (test mode: no real money moves).</p>
        <div>
          <button type="button" onClick={buy} disabled={pending}>
            {pending ? "Opening checkout…" : "Buy Nothing to See Here"}
          </button>
        </div>
        {error && <p className={styles.warning}>Checkout failed: {error}</p>}
      </div>
    </section>
  );
}
