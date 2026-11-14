import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Navigate, useLocation, useNavigate } from "react-router";
import { api } from "../api/client";
import { useSession } from "../api/queries";
import { ErrorMessage, Loading } from "../components/Feedback";
import { useCopy } from "../copy/CopyProvider";
import styles from "./pages.module.css";

export function LoginPage() {
  const copy = useCopy();
  const session = useSession();
  const client = useQueryClient();
  const navigate = useNavigate();
  const location = useLocation();
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [signingUp, setSigningUp] = useState(false);
  const [created, setCreated] = useState(false);
  const signup = useMutation({ mutationFn: () => api.signup(username, password), onSuccess: () => { setSigningUp(false); setCreated(true); setPassword(""); } });
  const login = useMutation({ mutationFn: () => api.login(username, password), onSuccess: (user) => {
    client.setQueryData(["session"], user);
    const from = (location.state as { from?: string } | null)?.from;
    navigate(from?.startsWith("/inbox") ? from : "/inbox", { replace: true });
  }});
  if (session.isPending) return <Loading />;
  if (session.data) return <Navigate to="/inbox" replace />;
  return <section className={styles.page}>
    <h1>{copy.auth.title}</h1>
    <form className={`${styles.card} ${styles.form}`} onSubmit={(event) => { event.preventDefault(); if (signingUp) signup.mutate(); else login.mutate(); }}>
      <label className={styles.field}>{copy.auth.username}<input autoComplete="username" required value={username} onChange={(event) => setUsername(event.target.value)} /></label>
      <label className={styles.field}>{copy.auth.password}<input type="password" autoComplete="current-password" required value={password} onChange={(event) => setPassword(event.target.value)} /></label>
      {signingUp && <p className={styles.hint}>{copy.auth.passwordHint}</p>}
      {created && !signingUp && <p role="status">{copy.auth.created}</p>}
      {(signingUp ? signup.error : login.error) && <ErrorMessage error={signingUp ? signup.error : login.error} />}
      <button type="submit" disabled={login.isPending || signup.isPending}>{signingUp ? copy.auth.signup : copy.auth.login}</button>
      <button type="button" onClick={() => { setSigningUp(!signingUp); setCreated(false); login.reset(); signup.reset(); }}>{signingUp ? copy.auth.backToLogin : copy.auth.signup}</button>
    </form>
  </section>;
}
