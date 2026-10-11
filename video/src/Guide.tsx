// How to use Mimir: ~154 s at 100 BPM on the film's clock and look, scored by score/guide.py (public/guide.wav).
import { Audio } from '@remotion/media'
import { AbsoluteFill, staticFile } from 'remotion'
import { C, F, Grain, Scene, Well } from './lib'
import { Rail } from './guide/kit'
import { Account, Get, Hello, Intro, Library, Name, Plug, Pocket, Setup, What } from './guide/setup'
import { Ask, Correct, Daily, Okay, Plan, Quick, Tap, Watch } from './guide/daily'
import { Always, End, Hush, Inbox, Notes, Recap } from './guide/more'

export const Guide: React.FC = () => (
  <AbsoluteFill style={{ background: C.deep, fontFamily: F.sans }}>
    <Well />
    <Scene id="intro" exit={0.75}><Intro /></Scene>
    <Scene id="what"><What /></Scene>
    <Scene id="setup"><Setup /></Scene>
    <Scene id="get"><Get /></Scene>
    <Scene id="account" exit={0}><Account /></Scene>
    <Scene id="name"><Name /></Scene>
    <Scene id="hello"><Hello /></Scene>
    <Scene id="pocket"><Pocket /></Scene>
    <Scene id="plug"><Plug /></Scene>
    <Scene id="library"><Library /></Scene>
    <Scene id="daily"><Daily /></Scene>
    <Scene id="ask"><Ask /></Scene>
    <Scene id="plan"><Plan /></Scene>
    <Scene id="watch"><Watch /></Scene>
    <Scene id="tap"><Tap /></Scene>
    <Scene id="okay"><Okay /></Scene>
    <Scene id="quick"><Quick /></Scene>
    <Scene id="correct"><Correct /></Scene>
    <Scene id="notes"><Notes /></Scene>
    <Scene id="hush" exit={0.25}><Hush /></Scene>
    <Scene id="inbox"><Inbox /></Scene>
    <Scene id="always"><Always /></Scene>
    <Scene id="recap" exit={0}><Recap /></Scene>
    <Scene id="end" exit={0} drift={0.04}><End /></Scene>
    <Rail />
    <Grain />
    <Audio src={staticFile('guide.wav')} />
  </AbsoluteFill>
)
