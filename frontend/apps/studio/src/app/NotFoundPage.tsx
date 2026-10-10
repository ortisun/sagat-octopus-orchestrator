import { Link } from 'react-router';

export function NotFoundPage() {
  return (
    <section>
      <h1>Page not found</h1>
      <p>
        <Link to="/workflows">Go to the workflows</Link>
      </p>
    </section>
  );
}
