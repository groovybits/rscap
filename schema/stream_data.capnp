@0x8a218fdfd03af08e;

struct StreamData {
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
    startTime @16 :UInt64;
    totalBits @17 :UInt64;
    count @18 :UInt32;
    rtpTimestamp @19 :UInt32;
    rtpPayloadType @20 :UInt8;
    rtpPayloadTypeName @21 :Text;
    rtpLineNumber @22 :UInt16;
    rtpLineOffset @23 :UInt16;
    rtpLineLength @24 :UInt16;
    rtpFieldId @25 :UInt8;
    rtpLineContinuation @26 :UInt8;
    rtpExtendedSequenceNumber @27 :UInt16;
}
