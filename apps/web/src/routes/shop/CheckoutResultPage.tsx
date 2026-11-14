import { useQuery } from "@tanstack/react-query";
import { Link, useSearchParams } from "react-router";
import { api } from "../../api/client";
import styles from "../pages.module.css";

/** Stripe sends the buyer back here. We ask the API (which asks Stripe) whether it was paid. */
export function CheckoutSuccessPage() {
  const [params] = useSearchParams();
  const sessionId = params.get("session_id") ?? "";
  const status = useQuery({
    queryKey: ["checkout", sessionId],
    queryFn: () => api.getCheckout(sessionId),
    enabled: sessionId !== "",
  });
  const paid = status.data?.payment_status === "paid";

  return (
    <section className={styles.page}>
      <h1>{paid ? "Thank you! Nothing has been purchased." : "Checking your payment…"}</h1>
      <div className={styles.card}>
        {status.isError && <p className={styles.warning}>Could not confirm the payment yet.</p>}
        {paid && <p>Your payment went through. There is still nothing to see here, as promised.</p>}
        <Link to="/">← Back to Nullbox</Link>
      </div>
    </section>
  );
}

export function CheckoutCancelPage() {
  return (
    <section className={styles.page}>
      <h1>Checkout cancelled</h1>
      <div className={styles.card}>
        <p>No payment was taken.</p>
        <Link to="/upgrade">← Try again</Link>
      </div>
    </section>
  );
}
