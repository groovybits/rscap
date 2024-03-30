@0x8a218fdfd03af08e;

struct StreamDataCapnp {
    pid @0 :UInt16;
    pmtPid @1 :UInt16;
    programNumber @2 :UInt16;
    streamType @3 :Text;
    continuityCounter @4 :UInt8;
    timestamp @5 :UInt64;
    bitrate @6 :UInt32;
    bitrateMax @7 :UInt32;
    bitrateMin @8 :UInt32;
    bitrateAvg @9 :UInt32;
    iat @10 :UInt64;
    iatMax @11 :UInt64;
    iatMin @12 :UInt64;
    iatAvg @13 :UInt64;
    errorCount @14 :UInt32;
    lastArrivalTime @15 :UInt64;
    lastSampleTime @16 :UInt64;
    startTime @17 :UInt64;
    totalBits @18 :UInt64;
    count @19 :UInt32;
    rtpTimestamp @20 :UInt32;
    rtpPayloadType @21 :UInt8;
    rtpPayloadTypeName @22 :Text;
    rtpLineNumber @23 :UInt16;
    rtpLineOffset @24 :UInt16;
    rtpLineLength @25 :UInt16;
    rtpFieldId @26 :UInt8;
    rtpLineContinuation @27 :UInt8;
    rtpExtendedSequenceNumber @28 :UInt16;
    streamTypeNumber @29 :UInt8;
}
