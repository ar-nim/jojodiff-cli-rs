#!/bin/bash
mv c:/temp/jdiff.txt jdiff$1.txt
grep '^Hash Pnt' jdiff$1.txt | cut -c22-27 | sort -n | uniq -c >jdst$1.txt
