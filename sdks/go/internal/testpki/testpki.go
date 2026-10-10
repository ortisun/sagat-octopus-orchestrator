// Package testpki makes throw-away certificates for the tests of the TLS paths: a certificate
// authority and the server and client certificates it signs. ECDSA P-256 keys, valid for an hour,
// generated in memory and never written anywhere. It is for tests only.
package testpki

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"math/big"
	"net"
	"testing"
	"time"
)

// Authority is a certificate authority.
type Authority struct {
	Cert *x509.Certificate
	// Pool trusts this authority and nothing else.
	Pool *x509.CertPool
	key  *ecdsa.PrivateKey
}

func serial(t testing.TB) *big.Int {
	t.Helper()
	n, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 120))
	if err != nil {
		t.Fatal(err)
	}
	return n
}

// NewAuthority makes a self-signed certificate authority.
func NewAuthority(t testing.TB, commonName string) *Authority {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	template := &x509.Certificate{
		SerialNumber:          serial(t),
		Subject:               pkix.Name{CommonName: commonName},
		NotBefore:             time.Now().Add(-time.Hour),
		NotAfter:              time.Now().Add(time.Hour),
		KeyUsage:              x509.KeyUsageCertSign | x509.KeyUsageCRLSign,
		BasicConstraintsValid: true,
		IsCA:                  true,
	}
	der, err := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if err != nil {
		t.Fatal(err)
	}
	cert, err := x509.ParseCertificate(der)
	if err != nil {
		t.Fatal(err)
	}
	pool := x509.NewCertPool()
	pool.AddCert(cert)
	return &Authority{Cert: cert, Pool: pool, key: key}
}

// Server issues a certificate for a TLS server that answers to the given names (DNS names or IP
// addresses).
func (a *Authority) Server(t testing.TB, commonName string, hosts ...string) tls.Certificate {
	t.Helper()
	return a.issue(t, commonName, x509.ExtKeyUsageServerAuth, hosts)
}

// Client issues a certificate for a TLS client (mTLS).
func (a *Authority) Client(t testing.TB, commonName string) tls.Certificate {
	t.Helper()
	return a.issue(t, commonName, x509.ExtKeyUsageClientAuth, nil)
}

func (a *Authority) issue(t testing.TB, commonName string, usage x509.ExtKeyUsage, hosts []string) tls.Certificate {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	template := &x509.Certificate{
		SerialNumber: serial(t),
		Subject:      pkix.Name{CommonName: commonName},
		NotBefore:    time.Now().Add(-time.Hour),
		NotAfter:     time.Now().Add(time.Hour),
		KeyUsage:     x509.KeyUsageDigitalSignature,
		ExtKeyUsage:  []x509.ExtKeyUsage{usage},
	}
	for _, host := range hosts {
		if ip := net.ParseIP(host); ip != nil {
			template.IPAddresses = append(template.IPAddresses, ip)
		} else {
			template.DNSNames = append(template.DNSNames, host)
		}
	}
	der, err := x509.CreateCertificate(rand.Reader, template, a.Cert, &key.PublicKey, a.key)
	if err != nil {
		t.Fatal(err)
	}
	return tls.Certificate{Certificate: [][]byte{der}, PrivateKey: key}
}
