// Mimir, the film: ~127 s at 100 BPM, every cut on a beat of the score (public/score.wav, from score/score.py).
import { Audio } from '@remotion/media'
import { AbsoluteFill, staticFile } from 'remotion'
import { C, F, Grain, Scene, Well } from './lib'
import { Cold, Outro, Title } from './scenes/open'
import { Channels, Chat, How, Memory } from './scenes/product'
import { Approve, Bots, Meetings, Routines, Skills, Sources } from './scenes/features'
import { Computer, Server, Teach } from './scenes/server'
import { Ai, Everywhere, Yours } from './scenes/finale'

export const Film: React.FC = () => (
  <AbsoluteFill style={{ background: C.deep, fontFamily: F.sans }}>
    <Well />
    <Scene id="cold"><Cold /></Scene>
    <Scene id="title" exit={0.75}><Title /></Scene>
    <Scene id="how"><How /></Scene>
    <Scene id="chat" exit={0}><Chat /></Scene>
    <Scene id="channels"><Channels /></Scene>
    <Scene id="memory"><Memory /></Scene>
    <Scene id="routines"><Routines /></Scene>
    <Scene id="sources"><Sources /></Scene>
    <Scene id="approve"><Approve /></Scene>
    <Scene id="skills"><Skills /></Scene>
    <Scene id="bots"><Bots /></Scene>
    <Scene id="meetings"><Meetings /></Scene>
    <Scene id="server"><Server /></Scene>
    <Scene id="computer" exit={0} drift={0}><Computer /></Scene>
    <Scene id="teach" drift={0}><Teach /></Scene>
    <Scene id="ai" exit={0.25}><Ai /></Scene>
    <Scene id="yours" exit={0.25}><Yours /></Scene>
    <Scene id="everywhere" exit={0}><Everywhere /></Scene>
    <Scene id="outro" exit={0} drift={0.04}><Outro /></Scene>
    <Grain />
    <Audio src={staticFile('score.wav')} />
  </AbsoluteFill>
)
