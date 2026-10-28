// Unknown routes (and `/drafts` in M1, D23). W3 owns it (docs/ARCHITECTURE.md §9.1).
import { Link } from 'react-router';
import { PlaceholderPage } from './PlaceholderPage';

export default function NotFound() {
  return (
    <PlaceholderPage title="Page not found">
      <Link to="/home">Go to Home</Link>
    </PlaceholderPage>
  );
}
