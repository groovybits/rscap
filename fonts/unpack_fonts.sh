#!/bin/bash
#
#
# Fonts from https://corefonts.sourceforge.net/

wget https://www.cabextract.org.uk/cabextract-1.11.tar.gz
tar xf cabextract-1.11.tar.gz
cd cabextract-1.11
./configure --prefix `pwd`
make
cd ../

cabextract-1.11/cabextract --lowercase --directory=cab-contents trebuc32.exe


cp -f cab-contents/trebuc.ttf TrebuchetMS.ttf
cp -f cab-contents/trebucbd.ttf TrebuchetMSBold.ttf
rm -rf cab-contents

