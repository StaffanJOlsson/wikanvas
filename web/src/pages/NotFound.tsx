// Unknown routes (and `/drafts` in M1, D23). W3 owns it (docs/ARCHITECTURE.md §9.1).
import { Link } from 'react-router';
import { SidebarToggle } from '../components/topbar/SidebarToggle';

export default function NotFound() {
  return (
    <section className="placeholder-page">
      <SidebarToggle />
      <h1>Page not found</h1>
      <p>The page was moved, deleted, or never existed.</p>
      <p><Link to="/home">Go to Home</Link></p>
    </section>
  );
}
