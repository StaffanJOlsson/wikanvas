// Placeholder page (W0b); W3 implements the sections (docs/ARCHITECTURE.md §9.1, §9.6.6).
import { useParams } from 'react-router';
import NotFound from './NotFound';
import { PlaceholderPage } from './PlaceholderPage';

export const SETTINGS_SECTIONS = ['account', 'appearance', 'workspace', 'tokens', 'data', 'about'] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export function isSettingsSection(s: string | undefined): s is SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(s ?? '');
}

const titles: Record<SettingsSection, string> = {
  account: 'Account',
  appearance: 'Appearance',
  workspace: 'Workspace',
  tokens: 'API tokens',
  data: 'Data',
  about: 'About',
};

export default function Settings() {
  const { section } = useParams();
  if (!isSettingsSection(section)) return <NotFound />;
  return <PlaceholderPage title={`Settings: ${titles[section]}`}>This settings section comes with W3.</PlaceholderPage>;
}
