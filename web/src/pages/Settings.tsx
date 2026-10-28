// Settings (docs/ARCHITECTURE.md §9.1, §9.6.6): Account, Appearance, Workspace, API tokens, Data and About, one route
// each (`/settings/:section`). An unknown section is NotFound.
import type { ComponentType } from 'react';
import { NavLink, useParams } from 'react-router';
import { PageHeader } from '../components/topbar/PageHeader';
import NotFound from './NotFound';
import { About } from './settings/About';
import { Account } from './settings/Account';
import { ApiTokens } from './settings/ApiTokens';
import { Appearance } from './settings/Appearance';
import { DataSettings } from './settings/DataSettings';
import { WorkspaceSettings } from './settings/WorkspaceSettings';

export const SETTINGS_SECTIONS = ['account', 'appearance', 'workspace', 'tokens', 'data', 'about'] as const;
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export function isSettingsSection(s: string | undefined): s is SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(s ?? '');
}

export const SETTINGS_TITLES: Record<SettingsSection, string> = {
  account: 'Account',
  appearance: 'Appearance',
  workspace: 'Workspace',
  tokens: 'API tokens',
  data: 'Data',
  about: 'About',
};

const bodies: Record<SettingsSection, ComponentType> = {
  account: Account,
  appearance: Appearance,
  workspace: WorkspaceSettings,
  tokens: ApiTokens,
  data: DataSettings,
  about: About,
};

export default function Settings() {
  const { section } = useParams();
  if (!isSettingsSection(section)) return <NotFound />;
  const Body = bodies[section];
  return (
    <div className="page settings-page">
      <PageHeader title={SETTINGS_TITLES[section]} />
      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings">
          {SETTINGS_SECTIONS.map((s) => (
            <NavLink key={s} to={`/settings/${s}`} className="settings-nav-link">{SETTINGS_TITLES[s]}</NavLink>
          ))}
        </nav>
        <div className="settings-body">
          <Body />
        </div>
      </div>
    </div>
  );
}
