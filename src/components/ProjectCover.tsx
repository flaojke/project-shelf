import { useId, useState } from 'react';
import androidCover from '../assets/covers/android-cover.png';
import webCover from '../assets/covers/web-cover.png';
import powershellCover from '../assets/covers/powershell-cover.png';
import dockerCover from '../assets/covers/docker-cover.png';
import type { Profile } from '../lib/types';

export function ProjectCover({ profile, className = '' }: { profile: Profile; className?: string }) {
  const uid = useId().replace(/:/g, '');
  const type = profile.kind === 'android' ? 'android' : profile.kind === 'powershell' ? 'site' : profile.kind === 'docker' ? 'docker' : 'web';
  const source = profile.kind === 'android' ? androidCover : profile.kind === 'powershell' ? powershellCover : profile.kind === 'docker' ? dockerCover : webCover;
  const [failedSource, setFailedSource] = useState<string | null>(null);
  if (failedSource !== source) return <div className={'project-cover cover-' + type + ' has-raster-cover ' + className}>
    <img src={source} alt="" decoding="async" onError={() => setFailedSource(source)} />
  </div>;
  return <div className={`project-cover cover-${type} ${className}`}>
    <svg viewBox="0 0 640 270" role="img" aria-label={`Авторская обложка ${profile.name}`}>
      <defs>
        <pattern id={`${uid}dots`} width="20" height="20" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="currentColor" opacity=".16"/></pattern>
        <filter id={`${uid}shadow`} x="-40%" y="-40%" width="180%" height="200%"><feDropShadow dx="0" dy="10" stdDeviation="13" floodColor={type === 'android' ? '#A65432' : type === 'web' ? '#315A44' : type === 'site' ? '#67516B' : '#376C8D'} floodOpacity=".13"/></filter>
      </defs>
      <rect width="640" height="270" fill="currentColor" opacity=".02"/>
      <rect width="640" height="270" fill={`url(#${uid}dots)`}/>
      {type === 'android' && <>
        <circle cx="553" cy="61" r="95" fill="#F5C49E" opacity=".48"/>
        <circle cx="58" cy="240" r="71" fill="#F2C29F" opacity=".4"/>
        <path d="M497 180a43 43 0 0 1 40 43" stroke="#CE7849" strokeWidth="1.5" fill="none"/>
        <g transform="translate(193 25) rotate(-7 126 110)" filter={`url(#${uid}shadow)`}>
          <rect width="254" height="226" rx="17" fill="#FFFDF8"/>
          <path d="M0 17Q0 0 17 0h220q17 0 17 17v42H0Z" fill="#DB754E"/>
          <path d="M62-7v24m130-24v24" stroke="#9C4A2E" strokeWidth="8" strokeLinecap="round"/>
          <text x="23" y="39" fill="white" fontFamily="Segoe UI,sans-serif" fontSize="17" fontWeight="600">ANDROID</text>
          <text x="230" y="39" fill="#FFE7D9" textAnchor="end" fontFamily="Segoe UI,sans-serif" fontSize="13">APP</text>
          {[0,1,2,3,4].map(row => [0,1,2,3,4,5,6].map(col => <g key={`${row}${col}`}><rect x={20+col*31} y={75+row*27} width="22" height="21" rx="6" fill={row===3&&col===4?'#DB754E':row===1&&col===2?'#F6E5D8':'transparent'}/><text x={31+col*31} y={90+row*27} textAnchor="middle" fontFamily="Segoe UI,sans-serif" fontSize="11" fill={row===3&&col===4?'white':'#826E5C'}>{(row*7+col)%31+1}</text></g>))}
        </g>
        <g transform="translate(438 162) rotate(9)" filter={`url(#${uid}shadow)`}><circle r="43" fill="#FFFDF8"/><circle r="35" fill="none" stroke="#F0DFCC"/><path d="M0-24v25l15 10" fill="none" stroke="#C86F47" strokeWidth="3" strokeLinecap="round"/><circle r="4" fill="#C86F47"/></g>
        <path d="m140 64 5 12 12 5-12 5-5 12-5-12-12-5 12-5Z" fill="#CB8252" opacity=".6"/>
      </>}
      {type === 'web' && <>
        <circle cx="588" cy="218" r="126" fill="#CFDFC5" opacity=".55"/>
        <circle cx="62" cy="42" r="80" fill="#D6E3CE" opacity=".75"/>
        <g stroke="#70936C" strokeWidth="2" fill="none"><path d="M320 82v41M180 123h280M180 123v40m140-40v40m140-40v40"/><circle cx="320" cy="123" r="4" fill="#70936C"/></g>
        <g filter={`url(#${uid}shadow)`}>
          <rect x="238" y="30" width="164" height="59" rx="12" fill="#507651"/>
          <rect x="112" y="158" width="136" height="72" rx="12" fill="#FFFDF8"/>
          <rect x="252" y="158" width="136" height="72" rx="12" fill="#FFFDF8"/>
          <rect x="392" y="158" width="136" height="72" rx="12" fill="#FFFDF8"/>
        </g>
        <g fontFamily="Segoe UI,sans-serif" textAnchor="middle"><text x="320" y="56" fill="#E2ECD9" fontSize="10" letterSpacing="2">WEB PROJECT</text><text x="320" y="76" fill="white" fontSize="14" fontWeight="600">Интерфейс и данные</text><text x="180" y="205" fill="#4C6745" fontSize="12">Страницы</text><text x="320" y="205" fill="#4C6745" fontSize="12">Сервисы</text><text x="460" y="205" fill="#4C6745" fontSize="12">Данные</text></g>
        <g stroke="#79906B" fill="none" strokeWidth="1.8"><rect x="172" y="174" width="15" height="13" rx="3"/><path d="m312 180 8-7 8 7-8 7z"/><ellipse cx="460" cy="176" rx="9" ry="3"/><path d="M451 176v10q9 6 18 0v-10"/></g>
        <path d="m537 58 7 7 15-18" stroke="#6F9164" strokeWidth="3" fill="none" strokeLinecap="round"/>
      </>}
      {type === 'site' && <>
        <circle cx="77" cy="240" r="125" fill="#DCD0E0" opacity=".6"/><circle cx="590" cy="14" r="110" fill="#E1D8E6"/>
        <g transform="translate(320 117)" strokeLinejoin="round">
          <path d="m-175 43 167-74 183 74-168 78Z" fill="#B8A1C5" stroke="#A98FB9"/>
          <path d="m-175 17 167-74 183 74-168 78Z" fill="#D4C4DF" stroke="#B8A0C7"/>
          <path d="m-175-9 167-74 183 74L7 69Z" fill="#FBF8FC" stroke="#CEBCD9"/>
          <path d="m-139-10 130-58 141 57L2 49Z" stroke="#E1D5E8" fill="none"/>
          <path d="m-98-28 133 54m-86-75 133 54m-200 16 131-58M-72 17 59-41" stroke="#E1D5E8" fill="none"/>
          <path d="m-72-16 39-18 42 15 41-20 48 20-41 20-6 26-48 21-46-20-39 17-41-20Z" fill="#A58AB8" opacity=".57"/>
          <path d="m-31-33 41 16 38-18" stroke="#7F6095" strokeWidth="2" fill="none"/>
          <circle cx="10" cy="-18" r="6" fill="#7C598F"/><circle cx="50" cy="-37" r="4" fill="#A58AB8"/>
        </g>
        <g transform="translate(469 64)" filter={`url(#${uid}shadow)`}><rect width="76" height="65" rx="11" fill="#FFFBFF"/><path d="M18 41h8V29h10v12h10V20h10v21" fill="none" stroke="#9975AE" strokeWidth="3"/><path d="M16 49h44" stroke="#E2D5E9" strokeWidth="2"/></g>
        <path d="M110 72h24m-12-12v24" stroke="#AB94BA" strokeWidth="1.5"/>
      </>}
      {type === 'docker' && <>
        <circle cx="599" cy="240" r="120" fill="#BCD4E2" opacity=".6"/><circle cx="70" cy="25" r="102" fill="#D5E4EC" opacity=".7"/>
        <g stroke="#8EB5CA" strokeWidth="1.5" fill="none"><ellipse cx="320" cy="136" rx="174" ry="95" strokeDasharray="3 7"/><path d="m320 136-127-62m127 62 126-67m-126 67-153 52m153-52 145 58m-145-58 6 91m-6-91 9-100"/></g>
        <g filter={`url(#${uid}shadow)`}><circle cx="320" cy="136" r="49" fill="#407E9F"/><circle cx="193" cy="74" r="26" fill="#FFFDF9"/><circle cx="446" cy="69" r="31" fill="#FFFDF9"/><circle cx="167" cy="188" r="28" fill="#FFFDF9"/><circle cx="465" cy="194" r="27" fill="#FFFDF9"/><circle cx="326" cy="227" r="17" fill="#A8C6D6"/><circle cx="329" cy="36" r="12" fill="#9EBFD1"/></g>
        <g fill="none" strokeLinecap="round" strokeWidth="1.8"><g stroke="#EAF4F8"><path d="m299 126 21-12 21 12v23l-21 12-21-12zM299 126l21 12 21-12m-21 12v23"/></g><g stroke="#6C97AF"><rect x="183" y="66" width="20" height="15" rx="3"/><path d="M188 62v4m10-4v4m-10 15v4m10-4v4"/><circle cx="446" cy="69" r="12"/><path d="M434 69h24m-12-12q-10 12 0 24m0-24q10 12 0 24"/><path d="m157 191 7-10 7 7 9-14M455 185h20v17h-20zm0 6h20"/></g></g>
        <circle cx="238" cy="209" r="4" fill="#8DB1C5"/><circle cx="397" cy="120" r="4" fill="#8DB1C5"/>
      </>}
    </svg>
  </div>;
}
