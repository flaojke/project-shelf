import type { Profile } from '../lib/types';

const technologies: Record<Profile['kind'], readonly string[]> = {
  process: ['Vite', 'Node.js'],
  powershell: ['PowerShell', 'HTTP'],
  docker: ['Docker', 'Compose'],
  android: ['Android', 'ADB'],
};

export function TechnologyChips({ profile }: { profile: Profile }) {
  return <div className="technology-chips" role="list" aria-label="Технологии запуска">
    {technologies[profile.kind].map(technology => <span className="technology-chip" role="listitem" key={technology}>
      <span className="tech-dot" aria-hidden="true" />{technology}
    </span>)}
  </div>;
}
