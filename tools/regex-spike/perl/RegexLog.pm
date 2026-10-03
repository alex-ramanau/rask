package RegexLog;

# Loaded into Perl ack with -MRegexLog (see scripts/perl-ack-regex-log).
# Records every regex ack builds, and what it was built from, as JSON lines
# appended to $ENV{RASK_REGEX_LOG}.

use strict;
use warnings;

use Fcntl qw( :flock );
use JSON::PP ();
use re ();

use App::Ack ();
use App::Ack::Filter::Match ();
use App::Ack::Filter::FirstLineMatch ();

my $json = JSON::PP->new->canonical->ascii;

sub _log {
    my $rec = shift;

    my $file = $ENV{RASK_REGEX_LOG} or return;
    open( my $fh, '>>', $file ) or return;
    flock( $fh, LOCK_EX );
    print {$fh} $json->encode($rec), "\n";
    close $fh;

    return;
}

# [ pattern, modifiers ] for a qr//, or the plain string for a string.
sub _re {
    my $re = shift;
    return undef unless defined $re;
    return [ re::regexp_pattern($re) ] if ref $re eq 'Regexp';
    return "$re";
}

sub _wrap {
    my ( $name, $wrapper ) = @_;
    no strict 'refs';
    no warnings 'redefine';
    my $orig = \&{$name};
    *{$name} = sub { $wrapper->( $orig, @_ ) };
    return;
}

_wrap( 'App::Ack::build_regex' => sub {
    my ( $orig, $str, $opt ) = @_;
    my %opts = map { ($_ => ($opt->{$_} ? 1 : 0)) } qw( i w Q S );

    my @ret;
    my $died;
    {
        no warnings 'redefine';
        local *App::Ack::die = sub { $died = join( '', @_ ); CORE::die "rask-regexlog\n" };
        @ret = eval { $orig->( $str, $opt ) };
    }
    if ( defined $died ) {
        _log( { site => 'build_regex', input => $str, opts => \%opts, error => $died } );
        App::Ack::die( $died );
    }
    _log( { site => 'build_regex', input => $str, opts => \%opts, regex => _re($ret[0]), scan => _re($ret[1]) } );
    return @ret;
} );

_wrap( 'App::Ack::build_all_regexes' => sub {
    my ( $orig, $str, $opt ) = @_;
    my @ret = $orig->( $str, $opt );
    _log( {
        site   => 'build_all_regexes',
        input  => $str,
        and    => $opt->{and}, or => $opt->{or}, not => $opt->{not},
        match  => _re($ret[0]), not_re => _re($ret[1]),
        hilite => _re($ret[2]), scan   => _re($ret[3]),
    } );
    return @ret;
} );

for my $class ( qw( App::Ack::Filter::Match App::Ack::Filter::FirstLineMatch ) ) {
    _wrap( "${class}::new" => sub {
        my ( $orig, $cls, $re ) = @_;
        my $self = $orig->( $cls, $re );
        _log( { site => $class, input => $re, regex => _re($self->{regex}) } );
        return $self;
    } );
}

1;
