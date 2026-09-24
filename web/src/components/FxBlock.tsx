import { type JSX } from "solid-js";
import { FX_TYPE, FX_LABELS, type FxTypeId } from "../stores/tatum";
import {
  FilterBlock, SaturatorBlock, ChorusBlock, TiltEqBlock,
  CompBlock, DelayBlock, ReverbBlock, LimiterBlock,
  EqBlock, BitcrusherBlock, TapeStopBlock,
} from "./EffectsRack";

interface FxBlockProps {
  fxType: FxTypeId;
  color: string;
  params?: Record<string, number>;
  onParamChange?: (key: string, paramIdx: number, value: number) => void;
}

/** Renders the correct FX block component based on type. Just the block, no ports or chrome. */
export default function FxBlock(props: FxBlockProps): JSX.Element {
  const instanceProps = () => ({
    color: props.color,
    params: props.params,
    onParamChange: props.onParamChange,
  });

  switch (props.fxType) {
    case FX_TYPE.FILTER:        return <FilterBlock {...instanceProps()} />;
    case FX_TYPE.SATURATOR:     return <SaturatorBlock {...instanceProps()} />;
    case FX_TYPE.CHORUS:        return <ChorusBlock {...instanceProps()} />;
    case FX_TYPE.TILT_EQ:       return <TiltEqBlock {...instanceProps()} />;
    case FX_TYPE.COMPRESSOR:    return <CompBlock {...instanceProps()} />;
    case FX_TYPE.DELAY:         return <DelayBlock {...instanceProps()} />;
    case FX_TYPE.REVERB:        return <ReverbBlock {...instanceProps()} />;
    case FX_TYPE.LIMITER:       return <LimiterBlock {...instanceProps()} />;
    case FX_TYPE.THREE_BAND_EQ: return <EqBlock {...instanceProps()} />;
    case FX_TYPE.BITCRUSHER:    return <BitcrusherBlock {...instanceProps()} />;
    case FX_TYPE.TAPE_STOP:     return <TapeStopBlock {...instanceProps()} />;
    default:                    return <span>{FX_LABELS[props.fxType]}</span>;
  }
}
