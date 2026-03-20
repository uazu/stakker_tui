#!/usr/bin/perl -w

# Build up the BOX table, to copy/paste into `termout.rs`

# Values in data are codepoint and segments UDLR where each segment
# is: 0 empty, 1 normal, 2 thick, 3 double.
@data = qw(
    0020 0000
    2500 0011
    2501 0022
    2502 1100
    2503 2200
    250C 0101
    250D 0102
    250E 0201
    250F 0202
    2510 0110
    2511 0120
    2512 0210
    2513 0220
    2514 1001
    2515 1002
    2516 2001
    2517 2002
    2518 1010
    2519 1020
    251A 2010
    251B 2020
    251C 1101
    251D 1102
    251E 2101
    251F 1201
    2520 2201
    2521 2102
    2522 1202
    2523 2202
    2524 1110
    2525 1120
    2526 2110
    2527 1210
    2528 2210
    2529 2120
    252A 1220
    252B 2220
    252C 0111
    252D 0121
    252E 0112
    252F 0122
    2530 0211
    2531 0221
    2532 0212
    2533 0222
    2534 1011
    2535 1021
    2536 1012
    2537 1022
    2538 2011
    2539 2021
    253A 2012
    253B 2022
    253C 1111
    253D 1121
    253E 1112
    253F 1122
    2540 2111
    2541 1211
    2542 2211
    2543 2121
    2544 2112
    2545 1221
    2546 1212
    2547 2122
    2548 1222
    2549 2221
    254A 2212
    254B 2222
    2550 0033
    2551 3300
    2552 0103
    2553 0301
    2554 0303
    2555 0130
    2556 0310
    2557 0330
    2558 1003
    2559 3001
    255A 3003
    255B 1030
    255C 3010
    255D 3030
    255E 1103
    255F 3301
    2560 3303
    2561 1130
    2562 3310
    2563 3330
    2564 0133
    2565 0311
    2566 0333
    2567 1033
    2568 3011
    2569 3033
    256A 1133
    256B 3311
    256C 3333
    2574 0010
    2575 1000
    2576 0001
    2577 0100
    2578 0020
    2579 2000
    257A 0002
    257B 0200
    257C 0012
    257D 1200
    257E 0021
    257F 2100
    );

# Unicode codepoint to/from 8-bit representation
%c2b = ();
%b2c = ();

while (@data) {
    my $c = shift @data;
    my $bstr = shift @data;
    die unless $bstr =~ /^([0123])([0123])([0123])([0123])$/;
    my $b = ($1 << 6) + ($2 << 4) + ($3 << 2) + $4;
    die "Clash: $b $c $c2b{$c}" if exists $c2b{$c};
    die "Clash: $b $c $b2c{$b}" if exists $b2c{$b};
    $c2b{$c} = $b;
    $b2c{$b} = $c;
}

# Fixes to attempt in order
my @FIX = (
    # Single changes
    0x01, 0x04, 0x10, 0x40,
    # Double changes
    0x05, 0x11, 0x41, 0x14, 0x44, 0x50,
    # Triple changes
    0x15, 0x45, 0x51, 0x54,
    # Quadruple changes
    0x55);

# Fill in missing elements of the table by downgrading the character
# to something similar
my $missing = 0;
OUTER:
    for my $b (0..255) {
        next if exists $b2c{$b};

        my $lo = $b & 0x55;
        my $hi = ($b >> 1) & 0x55;
        my $double = $lo & $hi;
        my $thick = $hi & ~$lo;
        my $single = $lo & ~$hi;

        # Try downgrading double to thick line segments until we find
        # a match
        for $fix (@FIX) {
            my $mask = $fix * 3;
            if (($mask & $b) == $mask) {
                # All double, make them thick
                my $z = ($b & ~$mask) | ($fix * 2);
                if (exists $b2c{$z}) {
                    $b2c{$b} = $b2c{$z};
                    next OUTER;
                }
            }
        }

        printf("Missing: %02X\n", $b);
        $missing++;
}

die if $missing > 0;

print "pub const BOX: [char; 256] = [\n";
for my $b (0..255) {
    my $c = $b2c{$b};
    print " '\\u{$c}',";
    print "\n" if 7 == ($b & 7);
}
print "];\n";
