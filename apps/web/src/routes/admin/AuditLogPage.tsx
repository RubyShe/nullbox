import { useState } from "react";
import { useAuditLog } from "../../api/queries";
import { ErrorMessage, Loading } from "../../components/Feedback";
import { useCopy, useFormat } from "../../copy/CopyProvider";
import styles from "../pages.module.css";

const TOKEN_KEY = "nullbox.adminToken";

// Browser storage can be missing or throw (private windows), so the page works without it.
function loadToken(): string {
  try {
    return sessionStorage.getItem(TOKEN_KEY) ?? "";
  } catch {
    return "";
  }
}

function saveToken(token: string) {
  try {
    sessionStorage.setItem(TOKEN_KEY, token);
  } catch {
    // ignore
  }
}

export function AuditLogPage() {
  const copy = useCopy();
  const format = useFormat();
  const [token, setToken] = useState(loadToken);
  const [draft, setDraft] = useState("");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const log = useAuditLog(token, query);
  const columns = copy.audit.columns;

  return (
    <section className={styles.page}>
      <h1>{copy.audit.title}</h1>
      <p className={styles.intro}>{copy.audit.intro}</p>

      <form
        className={styles.filters}
        onSubmit={(event) => {
          event.preventDefault();
          const next = draft.trim() || token;
          saveToken(next);
          setToken(next);
          setQuery(search.trim());
        }}
      >
        <label>
          {copy.audit.adminToken}
          <input
            type="password"
            autoComplete="off"
            value={draft}
            placeholder={token ? "••••••••" : ""}
            onChange={(event) => setDraft(event.target.value)}
          />
        </label>
        <label>
          {copy.audit.search}
          <input
            type="search"
            value={search}
            placeholder={copy.audit.searchPlaceholder}
            onChange={(event) => setSearch(event.target.value)}
          />
        </label>
        <button type="submit">{copy.audit.search}</button>
      </form>

      {log.isLoading && token ? <Loading /> : null}
      {log.isError ? <ErrorMessage error={log.error} /> : null}
      {log.data && log.data.length === 0 ? <p className={styles.muted}>{copy.audit.empty}</p> : null}
      {log.data && log.data.length > 0 ? (
        <div className={styles.tableWrap}>
          <table className={styles.auditTable}>
            <thead>
              <tr>
                <th>{columns.time}</th>
                <th>{columns.kind}</th>
                <th>{columns.actor}</th>
                <th>{columns.ip}</th>
                <th>{columns.request}</th>
                <th>{columns.status}</th>
                <th>{columns.credential}</th>
                <th>{columns.summary}</th>
              </tr>
            </thead>
            <tbody>
              {log.data.map((entry) => {
                const refused = entry.status === 401 || entry.status === 403;
                return (
                  <tr key={entry.id} className={refused ? styles.refused : undefined}>
                    <td>{format.dateTime(entry.occurred_at)}</td>
                    <td>{entry.kind}</td>
                    <td>{entry.actor ?? "—"}</td>
                    <td>{entry.client_ip ?? "—"}</td>
                    <td>{entry.method ? `${entry.method} ${entry.path ?? ""}` : "—"}</td>
                    <td>
                      {entry.status ?? "—"}
                      {refused ? ` (${copy.audit.refusedBadge})` : ""}
                    </td>
                    <td>{entry.credential ? <code>{entry.credential}</code> : "—"}</td>
                    <td>{entry.summary}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : null}
    </section>
  );
}
